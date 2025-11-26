use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::config::REQUIRED_CONFIRMATIONS;
use crate::event_processor::EventProcessor;
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowFactory,
};
use crate::flows::btc_signature::btc_signature_subflow::{
    BtcSignatureSubFlowApi, BtcSignatureSubFlowFactoryApi,
};
use crate::flows::common::GlobalContext;
use crate::flows::pegout::pegout_flow::Steps;
use crate::flows::pegout::pegout_flow::{PegoutFlow, StepData};
use crate::types::{
    EventStatus, RegisterSignaturesBitVmxData, RskPegManagerEvents, TickScheduler, UserRequests,
};
use anyhow::anyhow;
use anyhow::{Context, Result, bail};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    OutgoingBitVMXApiMessages, PegOutAccepted, TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::TxIdParser;
use common::types::{BlockNumber, CommitteeId, Hash256, RskBlockAndUncles};
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use std::any::type_name_of_val;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use union_contracts::bindings::peg_manager::PegManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

pub const PEGOUT_ACCEPTED_NAME: &str = "pegout_accepted";
pub const BLOCKS_DELAY_FOR_TX_CHECK: u32 = 20;
pub const SPV_PROOF_MIN_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node

/// Processor that manages multiple pegout flow state machines
pub struct PegoutFlowProcessor<CG, BC, BSF, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    btc_sig_subflow_factory: FactoryBSF,
    pegout_flows: HashMap<Uuid, PegoutFlow<CG, BC>>,
    signature_flows: HashMap<Uuid, BSF>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<Uuid>,
}

impl<CG, BC>
    PegoutFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
    ) -> Self {
        let factory = BtcSignatureSubFlowFactory::new(contracts_gateway.clone(), rt_sync.clone());
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            btc_sig_subflow_factory: factory,
            pegout_flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            signature_flows: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
        }
    }

    pub fn get_user_take_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(&slot_index.to_be_bytes());
        hasher.update("user_take");

        // Get the result as a byte array
        let hash = hasher.finalize();
        let slice = hash
            .as_slice()
            .get(..16)
            .ok_or_else(|| anyhow!("SHA256 hash too short for UUID generation"))?;
        let uuid_bytes: [u8; 16] = slice
            .try_into()
            .context("Failed to convert hash slice to UUID bytes")?;
        Ok(Uuid::from_bytes(uuid_bytes))
    }

    /// Create a new flow for a PegoutRequested event
    pub fn create_flow_for_pegout_requested(&mut self, event: &PegoutRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.try_into()?;

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PegoutRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PegoutRequested event with committee id {}, as member I should respond",
            committee_id
        );

        let slot_index = event.slotId as usize;
        let committee_uuid: Uuid = Uuid::from_u128(event.committeeId.try_into()?);
        let flow_id = Self::get_user_take_pid(committee_uuid, slot_index)?;

        let mut flow = PegoutFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            flow_id,
            event.clone(),
        );

        // Initialize the flow with the PegoutRequested event
        flow.complete_step(StepData::PegoutRequested)?;

        self.pegout_flows.insert(flow_id, flow);

        info!(
            "Created new pegout flow {} for committee {}",
            flow_id, committee_id
        );
        Ok(())
    }

    /// Handle confirmed PegoutRegistered event
    fn handle_pegout_registered(
        &mut self,
        pr: &crate::types::EventWithBlock<PegoutRegistered>,
    ) -> Result<()> {
        info!("Processing confirmed PegoutRegistered event: {:?}", pr);
        // Find the flow corresponding to this pegout registration using event tx_hash with  flow.state.pegout_registered_tx
        let pegout_registered = pr.inner.clone();
        let pegout_registered_txid: Txid = TxIdParser::fb_32_to_txid(pegout_registered.txid);
        let flow_opt = self
            .pegout_flows
            .values_mut()
            .find(|flow| flow.get_user_take_txid() == Some(pegout_registered_txid));

        if let Some(flow) = flow_opt {
            flow.complete_step(StepData::PegoutRegistered(pegout_registered))?;
        } else {
            warn!(
                "No matching pegout flow found for PegoutRegistered event: {:?}",
                pr
            );
        }
        Ok(())
    }

    /// Clean up completed flows
    pub fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> = self
            .pegout_flows
            .iter()
            .filter(|(_, flow)| flow.is_done())
            .map(|(k, _)| *k)
            .collect();

        for internal_id in completed {
            debug!("Removing completed flow: {internal_id}");
            self.pegout_flows.remove(&internal_id);
        }
    }

    /// Process confirmed RSK events
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!("Processing confirmed RSK event: {:?}", event);

        match event {
            RskPegManagerEvents::PegoutRequested(pr) => {
                let committee_id = pr.inner.committeeId.try_into()?;
                if !self.global_context.my_committees().im_member(&committee_id) {
                    debug!(
                        "Handling PegoutRequested event with committee id {}, I am NOT member so I skip",
                        committee_id
                    );
                    return Ok(());
                }
                info!("Processing confirmed PegoutRequested event: {:?}", pr);
                self.create_flow_for_pegout_requested(&pr.inner)?;
            }
            RskPegManagerEvents::PegoutRegistered(pr) => {
                self.handle_pegout_registered(pr)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    /// Build event info for PegoutRequested events
    fn build_pegout_requested_event_info(
        event: &crate::types::EventWithBlock<PegoutRequested>,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegout-requested-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PegoutRequested(event.clone()),
        )
    }

    fn build_pegout_registered_event_info(
        event: &crate::types::EventWithBlock<PegoutRegistered>,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegout-registered-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PegoutRegistered(event.clone()),
        )
    }

    fn process_unhandled_confirmed_sig_flow_events(
        &mut self,
        block: &RskBlockAndUncles,
    ) -> Result<()> {
        let mut flows_to_dispatch = Vec::new();
        for (flow_id, signature_flow) in self.signature_flows.iter_mut() {
            signature_flow.delegate_block(block)?;
            if signature_flow.is_done() {
                flows_to_dispatch.push(*flow_id);
            }
        }

        for flow_id in &flows_to_dispatch {
            if let Some(flow) = self.pegout_flows.get_mut(flow_id) {
                flow.complete_step(StepData::DispatchTransaction)?;
                self.signature_flows.remove(&flow_id);
            } else {
                warn!(
                    "Signature flow done for unknown pegout flow_id: {}. Skipping dispatch step",
                    flow_id
                );
            }
        }

        Ok(())
    }

    fn handle_transaction_status_received(
        &mut self,
        flow_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        let flow = match self.pegout_flows.get_mut(&flow_id) {
            Some(flow) => flow,
            None => {
                trace!(
                    "Ignoring BitVMX Transaction event for unknown flow_id: {}",
                    flow_id
                );
                return Ok(());
            }
        };

        let TransactionStatus {
            tx_id,
            confirmations,
            ..
        } = tx_status;
        let flow_id = flow.flow_id();
        let expected_txid = flow
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        if expected_txid != tx_id {
            bail!(
                "Pegout state for flow_id: {} does not match received tx_id: {} from tx status message",
                flow_id,
                tx_id
            );
        }

        if flow.current_step() != Steps::ConfirmUserTakeTransaction {
            bail!(
                "Mismatch current step for flow {} expected {:?} having {:?}",
                flow_id,
                Steps::ConfirmUserTakeTransaction,
                flow.current_step()
            );
        }

        if confirmations >= SPV_PROOF_MIN_CONFIRMATIONS {
            debug!(
                "Transaction confirmed with sufficient confirmations for flow_id: {}",
                flow_id
            );
            flow.complete_step(StepData::TransactionConfirmed(tx_status))?;
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
        } else {
            debug!(
                "Transaction not confirmed with sufficient confirmations for flow_id: {}",
                flow_id
            );
            self.tx_status_scheduler
                .schedule(flow_id.clone(), BLOCKS_DELAY_FOR_TX_CHECK);
        }
        Ok(())
    }

    fn handle_transaction_status_tick(&mut self) -> Result<()> {
        if self.tx_status_scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.tx_status_scheduler.tick();
        for flow_id in ready {
            match self.pegout_flows.get_mut(&flow_id) {
                Some(flow) => {
                    if flow.current_step() == Steps::ConfirmUserTakeTransaction {
                        flow.request_transaction_status()?;
                    } else {
                        warn!(
                            "Mismatch current step for flow {} expected {:?} having {:?}",
                            flow_id,
                            Steps::ConfirmUserTakeTransaction,
                            flow.current_step()
                        );
                    }
                }
                None => {
                    warn!(
                        "Skipping delayed transaction status request for unknown flow {}",
                        flow_id
                    );
                }
            }
        }

        Ok(())
    }

    fn process_block_confirmations(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
            return Ok(());
        }

        self.blockchain_view.update(block.clone());

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter_map(|(key, event)| event.is_confirmed().then(|| key.clone()))
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                // properly cleanup the observer before processing the event
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {}: {}", key, e)
                }
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        // blocks allow periodic cleanup of completed flows, we can improve it with a cleanup task if needed
        self.cleanup_completed_flows();

        Ok(())
    }
}

impl<CG, BC> EventProcessor
    for PegoutFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn process_user_request(&mut self, _req: &UserRequests) -> Result<()> {
        // Pegout flows are created from RSK events, not from user requests
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        trace!("Processing BitVMX event: {:?}", event);

        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                trace!("Received CommInfo from BitVMX: {:?}", comm_info);
                //for any flow in flows having active step GetCommInfo, complete the step with the CommInfo
                for (flow_id, flow) in self.pegout_flows.iter_mut() {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Completing GetCommInfo step for flow {flow_id}");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGOUT_ACCEPTED_NAME) =>
            {
                info!(
                    "Received PegOutAccepted variable from BitVMX for flow_id: {}",
                    flow_id
                );
                debug!("PegOutAccepted data: {}", data);
                let input: PegOutAccepted = serde_json::from_str::<PegOutAccepted>(data)?;
                let flow = self
                    .pegout_flows
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id: {}", flow_id))?;
                if flow.current_step() != Steps::PrepareUserTakeSetup {
                    bail!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::PrepareUserTakeSetup,
                        flow.current_step()
                    );
                }
                // Note: v0.2.0 contracts - initSignatures is called with pegoutSignatureData.txid (the transaction ID),
                // not user_take_sighash. So we must use the txid from PegoutRequested event.
                let hash_to_sign = Hash256::from(flow.pegout_requested().pegoutSignatureData.txid);
                let register_input = RegisterSignaturesBitVmxData {
                    hash_to_sign,
                    nonce: input.user_take_nonce.clone(),
                    signature: input.user_take_signature.clone(),
                };
                flow.complete_step(StepData::PegoutAccepted(input))?;

                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(*flow_id);
                btc_sig_subflow.start_signature_flow(flow_id.clone(), &register_input)?;
                self.signature_flows
                    .insert(flow_id.clone(), btc_sig_subflow);
            }
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                if self.pegout_flows.contains_key(program_id) {
                    info!("Pegout setup was completed: flow_id={}", program_id);
                } else {
                    trace!(
                        "Ignoring BitVMX SetupCompleted for unknown program_id: {}",
                        program_id
                    );
                }
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                let spv_proof = spv_proof_opt.clone().ok_or_else(|| {
                    anyhow!("Received SPVProof event for tx_id {} without proof", tx_id)
                })?;

                let (flow_id, flow) =
                    match self.pegout_flows.iter_mut().find_map(|(flow_id, flow)| {
                        (flow.get_user_take_txid() == Some(*tx_id)).then_some((*flow_id, flow))
                    }) {
                        Some((flow_id, flow)) => (flow_id, flow),
                        None => {
                            debug!(
                                "Ignoring SPV proof for flow {} while at step {:?}",
                                "unknown", "unknown"
                            );
                            return Ok(());
                        }
                    };
                if flow.current_step() != Steps::RequestUserTakeSpvProof {
                    bail!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::RequestUserTakeSpvProof,
                        flow.current_step()
                    );
                } else {
                    flow.complete_step(StepData::SpvProof(spv_proof))?;
                }
            }
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            _ => {
                trace!("Ignoring BitVMX event: {:?}", event);
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::AllNoncesReady(data)
            | RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling signature event {:?}", data);
                for (flow_id, sig_flow) in self.signature_flows.iter_mut() {
                    sig_flow.delegate_rsk_event(*flow_id, event)?;
                }
                return Ok(());
            }
            _ => {
                //continue with the normal flow
            }
        }

        // useful for testing purposes
        if REQUIRED_CONFIRMATIONS == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PegoutRequested(e) => Self::build_pegout_requested_event_info(e),
            RskPegManagerEvents::PegoutRegistered(e) => Self::build_pegout_registered_event_info(e),
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending RSK event: {:?}", event);

            // properly clean up the observer before removing the event
            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}")
                }
            } else {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                REQUIRED_CONFIRMATIONS,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event
                .start_confirming(block_num)
                .context("Starting confirming")?;

            self.events_confirming
                .insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting for confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_transaction_status_tick()?;
        self.process_block_confirmations(block)?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutFlowProcessor");
        self.pegout_flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
    }
}
