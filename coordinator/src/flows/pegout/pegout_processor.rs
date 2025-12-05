use std::any::type_name_of_val;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    OutgoingBitVMXApiMessages, PegOutAccepted, TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{BlockNumber, CommitteeId, Hash256, RskBlockAndUncles, TxIdParser};
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use union_contracts::bindings::pegout_manager::PegoutManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::config::REQUIRED_CONFIRMATIONS;
use crate::event_processor::EventProcessor;
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi,
};
use crate::flows::common::GlobalContext;
use crate::flows::pegout::pegout_flow::{PegoutFlow, State, StepData, Steps};
use crate::store::{CoordinatorStoreApi, StoreKey, StorePrefix};
use crate::types::{
    EventStatus, RegisterSignaturesBitVmxData, RskPegManagerEvents, TickScheduler,
    TimeBasedScheduler, UserRequests,
};

pub const PEGOUT_ACCEPTED_NAME: &str = "pegout_accepted";
pub const BLOCKS_DELAY_FOR_TX_CHECK: u32 = 20;
pub const SPV_PROOF_MIN_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node
pub const ADVANCE_FUNDS_TIMEOUT_SECONDS: u64 = 60; // 1 minute for testing, in production should be 2 * 60 * 60 (2 hours)

/// Processor that manages multiple pegout flow state machines
pub struct PegoutFlowProcessor<CG, BC, BSF, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    BSF: BtcSignatureSubFlowApi,
    FactoryBSF: BtcSignatureSubFlowFactoryApi<BSF>,
    S: CoordinatorStoreApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    btc_sig_subflow_factory: FactoryBSF,
    pegout_flows: HashMap<Uuid, PegoutFlow<CG, BC, S>>,
    signature_flows: HashMap<Uuid, BSF>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<Uuid>,
    advance_funds_timeout_scheduler: TimeBasedScheduler<Uuid>,
    flows_pending_timeout: HashSet<Uuid>, // Flows that need timeout scheduled on next block
    store: Rc<S>,
}

impl<CG, BC, S>
    PegoutFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: Rc<S>,
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
            advance_funds_timeout_scheduler: TimeBasedScheduler::new(),
            flows_pending_timeout: HashSet::new(),
            store,
        }
    }

    pub fn restore_or_new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: Rc<S>,
    ) -> Result<Self> {
        let mut processor =
            Self::new(contracts_gateway, rt_sync, bitvmx_broker, global_context, store);
        processor.restore_flows_from_store()?;
        Ok(processor)
    }

    // todo(fede) this is probably same code as pegin restore flow
    fn restore_flows_from_store(&mut self) -> Result<()> {
        debug!("Checking for pegout flows to restore from persistence");

        let saved_flows: HashMap<Uuid, State> =
            self.store.load_all_flows(&StorePrefix::PegoutFlow)?;

        for (id, saved_state) in &saved_flows {
            self.restore_flow(id, saved_state);
        }

        if !self.pegout_flows.is_empty() {
            info!("Restored {} pegout flows from persistence", self.pegout_flows.len());
        }

        Ok(())
    }

    // restores a flow from a saved state and return a reference to it
    fn restore_flow(&mut self, id: &Uuid, saved_state: &State) {
        let flow = PegoutFlow::from_saved_state(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            saved_state.clone(),
            Rc::clone(&self.store),
        );
        info!("Restoring pegout flow {id} at step {:?}", &flow.current_step(),);
        self.pegout_flows.insert(*id, flow);

        debug!("Restored flow {id}");
    }

    pub fn get_user_take_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(slot_index.to_be_bytes());
        hasher.update("user_take");

        // Get the result as a byte array
        let hash = hasher.finalize();
        let slice = hash
            .as_slice()
            .get(..16)
            .ok_or_else(|| anyhow!("SHA256 hash too short for UUID generation"))?;
        let uuid_bytes: [u8; 16] =
            slice.try_into().context("Failed to convert hash slice to UUID bytes")?;
        Ok(Uuid::from_bytes(uuid_bytes))
    }

    /// Create a new flow for a `PegoutRequested` event
    pub fn create_flow_for_pegout_requested(&mut self, event: &PegoutRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.try_into()?;

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PegoutRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PegoutRequested event with committee id {committee_id}, as member I should respond"
        );

        let slot_index = usize::try_from(event.slotId)
            .map_err(|_| anyhow!("slotId {} too large for usize", event.slotId))?;
        let committee_uuid: Uuid = Uuid::from_u128(event.committeeId.try_into()?);
        let flow_id = Self::get_user_take_pid(committee_uuid, slot_index)?;

        let mut flow = PegoutFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            flow_id,
            event,
            Rc::clone(&self.store),
        );

        // Initialize the flow with the PegoutRequested event
        flow.complete_step(&StepData::PegoutRequested)?;

        self.pegout_flows.insert(flow_id, flow);

        info!("Created new pegout flow {flow_id} for committee {committee_id}");
        Ok(())
    }

    /// Handle confirmed `PegoutRegistered` event
    fn handle_pegout_registered(
        &mut self,
        pr: &crate::types::EventWithBlock<PegoutRegistered>,
    ) -> Result<()> {
        info!("Processing confirmed PegoutRegistered event: {pr:?}");
        // Find the flow corresponding to this pegout registration using event tx_hash with  flow.state.pegout_registered_tx
        let pegout_registered = pr.inner.clone();
        let pegout_registered_txid: Txid = TxIdParser::fb_32_to_txid(pegout_registered.txid);
        let flow_opt = self
            .pegout_flows
            .values_mut()
            .find(|flow| flow.get_user_take_txid() == Some(pegout_registered_txid));

        if let Some(flow) = flow_opt {
            flow.complete_step(&StepData::PegoutRegistered(pegout_registered))?;
        } else {
            warn!("No matching pegout flow found for PegoutRegistered event: {pr:?}");
        }
        Ok(())
    }

    /// Clean up completed flows
    pub fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> =
            self.pegout_flows.iter().filter(|(_, flow)| flow.is_done()).map(|(k, _)| *k).collect();

        for internal_id in &completed {
            debug!("Removing completed flow: {internal_id}");
            self.pegout_flows.remove(internal_id);

            self.store.delete_flow(&StoreKey::PegoutFlow(*internal_id)).unwrap_or_else(|e| {
                error!("Failed to remove completed flow {internal_id} from persistence: {e}");
            });
        }
    }

    /// Process confirmed RSK events
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!("Processing confirmed RSK event: {event:?}");

        match event {
            RskPegManagerEvents::PegoutRequested(pr) => {
                let committee_id = pr.inner.committeeId.try_into()?;
                if !self.global_context.my_committees().im_member(&committee_id) {
                    debug!(
                        "Handling PegoutRequested event with committee id {committee_id}, I am NOT member so I skip"
                    );
                    return Ok(());
                }
                info!("Processing confirmed PegoutRequested event: {pr:?}");
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

    /// Build event info for `PegoutRequested` events
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
        for (flow_id, signature_flow) in &mut self.signature_flows {
            signature_flow.delegate_block(block)?;
            if signature_flow.is_done() {
                flows_to_dispatch.push(*flow_id);
            }
        }

        for flow_id in &flows_to_dispatch {
            if let Some(flow) = self.pegout_flows.get_mut(flow_id) {
                flow.complete_step(&StepData::DispatchTransaction)?;
                self.signature_flows.remove(flow_id);

                // Cancel advance funds timeout since signatures completed successfully
                if self.advance_funds_timeout_scheduler.is_scheduled(flow_id) {
                    debug!(
                        "Cancelling advance funds timeout for flow_id: {flow_id} - signatures completed",
                    );
                    self.advance_funds_timeout_scheduler.cancel(flow_id);
                }
            } else {
                warn!(
                    "Signature flow done for unknown pegout flow_id: {flow_id}. Skipping dispatch step"
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
        let Some(flow) = self.pegout_flows.get_mut(flow_id) else {
            trace!("Ignoring BitVMX Transaction event for unknown flow_id: {flow_id}");
            return Ok(());
        };

        let TransactionStatus { tx_id, confirmations, .. } = tx_status;
        let flow_id = flow.flow_id();
        let expected_txid = flow
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        if expected_txid != tx_id {
            bail!(
                "Pegout state for flow_id: {flow_id} does not match received tx_id: {tx_id} from tx status message"
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
            debug!("Transaction confirmed with sufficient confirmations for flow_id: {flow_id}");
            flow.complete_step(&StepData::TransactionConfirmed(tx_status))?;
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
        } else {
            debug!(
                "Transaction not confirmed with sufficient confirmations for flow_id: {flow_id}"
            );
            self.tx_status_scheduler.schedule(flow_id, BLOCKS_DELAY_FOR_TX_CHECK);
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
                    warn!("Skipping delayed transaction status request for unknown flow {flow_id}");
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

        self.blockchain_view.update(block);

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter(|(_, event)| event.is_confirmed())
            .map(|(key, _)| key.clone())
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                // properly cleanup the observer before processing the event
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {key}: {e}");
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

    /// Schedule timeouts for flows that received `pegout_accepted` but didn't have block timestamp yet
    fn schedule_pending_timeouts(&mut self, block: &RskBlockAndUncles) {
        if self.flows_pending_timeout.is_empty() {
            return;
        }

        let current_timestamp = block.block().timestamp().value();
        let pending_flows: Vec<Uuid> = self.flows_pending_timeout.iter().copied().collect();

        for flow_id in pending_flows {
            if let Some(flow) = self.pegout_flows.get(&flow_id) {
                // Only schedule if flow is still waiting for signatures (not yet dispatched)
                if flow.current_step() == Steps::DispatchTransaction {
                    self.advance_funds_timeout_scheduler.schedule(
                        flow_id,
                        current_timestamp,
                        ADVANCE_FUNDS_TIMEOUT_SECONDS,
                    );
                    info!(
                        "Scheduled advance funds timeout for flow_id: {} at timestamp: {} (expires at: {})",
                        flow_id,
                        current_timestamp,
                        current_timestamp + ADVANCE_FUNDS_TIMEOUT_SECONDS
                    );
                }
            }
            self.flows_pending_timeout.remove(&flow_id);
        }
    }

    /// Check for expired advance funds timeouts and trigger operator take
    fn handle_advance_funds_timeout_expired(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.advance_funds_timeout_scheduler.is_empty() {
            return Ok(());
        }

        let current_timestamp = block.block().timestamp().value();
        let expired_flows = self.advance_funds_timeout_scheduler.check_expired(current_timestamp);

        for flow_id in expired_flows {
            info!(
                "Advance funds timeout expired for flow_id: {flow_id} at timestamp: {current_timestamp}",
            );
            self.trigger_operator_take_for_flow(flow_id)?;
        }

        Ok(())
    }

    /// Trigger operator take for a flow when timeout expires
    /// This completes the `DispatchTransaction` step with `TriggerOperatorTakeTimeout` data
    fn trigger_operator_take_for_flow(&mut self, flow_id: Uuid) -> Result<()> {
        let flow = self
            .pegout_flows
            .get_mut(&flow_id)
            .ok_or_else(|| anyhow!("Flow not found for flow_id: {flow_id}"))?;

        // Verify flow is still in the expected state
        if flow.current_step() != Steps::DispatchTransaction {
            warn!(
                "Cannot trigger operator take for flow_id: {} - flow is at step {:?}, expected {:?}",
                flow_id,
                flow.current_step(),
                Steps::DispatchTransaction
            );
            return Ok(());
        }

        info!(
            "Timeout expired for flow_id: {flow_id}, completing DispatchTransaction step with TriggerOperatorTakeTimeout",
        );

        // Complete the DispatchTransaction step with timeout data
        // This will transition to TriggerOperatorTake step, which will call trigger_operator_take
        flow.complete_step(&StepData::TriggerOperatorTakeTimeout)?;

        // After trigger_operator_take completes in start_step, complete TriggerOperatorTake step
        // to transition to Done
        if flow.current_step() == Steps::TriggerOperatorTake {
            flow.complete_step(&StepData::TriggerOperatorTakeTimeout)?;
        }

        Ok(())
    }
}

impl<CG, BC, S> EventProcessor
    for PegoutFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn process_user_request(&mut self, _req: &UserRequests) -> Result<()> {
        // Pegout flows are created from RSK events, not from user requests
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        trace!("Processing BitVMX event: {event:?}");

        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                trace!("Received CommInfo from BitVMX: {comm_info:?}");
                //for any flow in flows having active step GetCommInfo, complete the step with the CommInfo
                for (flow_id, flow) in &mut self.pegout_flows {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Completing GetCommInfo step for flow {flow_id}");
                        flow.complete_step(&StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGOUT_ACCEPTED_NAME) =>
            {
                info!("Received PegOutAccepted variable from BitVMX for flow_id: {flow_id}");
                debug!("PegOutAccepted data: {data}");
                let input: PegOutAccepted = serde_json::from_str::<PegOutAccepted>(data)?;
                let flow = self
                    .pegout_flows
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id: {flow_id}"))?;
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
                    signature: input.user_take_signature,
                };
                flow.complete_step(&StepData::PegoutAccepted(input))?;

                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(*flow_id);
                btc_sig_subflow.start_signature_flow(*flow_id, &register_input)?;
                self.signature_flows.insert(*flow_id, btc_sig_subflow);

                // Schedule advance funds timeout: 2 hours from now
                // We'll schedule it when we process the next block with its timestamp
                info!(
                    "Pegout accepted for flow_id: {flow_id}, will schedule advance funds timeout on next block",
                );
                self.flows_pending_timeout.insert(*flow_id);
            }
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                if self.pegout_flows.contains_key(program_id) {
                    info!("Pegout setup was completed: flow_id={program_id}");
                } else {
                    trace!("Ignoring BitVMX SetupCompleted for unknown program_id: {program_id}");
                }
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                let spv_proof = spv_proof_opt.clone().ok_or_else(|| {
                    anyhow!("Received SPVProof event for tx_id {tx_id} without proof")
                })?;

                let Some((flow_id, flow)) =
                    self.pegout_flows.iter_mut().find_map(|(flow_id, flow)| {
                        (flow.get_user_take_txid() == Some(*tx_id)).then_some((*flow_id, flow))
                    })
                else {
                    trace!("Ignoring SPV proof for tx_id {tx_id} without matching flow");
                    return Ok(());
                };
                if flow.current_step() != Steps::RequestUserTakeSpvProof {
                    bail!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::RequestUserTakeSpvProof,
                        flow.current_step()
                    );
                }
                flow.complete_step(&StepData::SpvProof(spv_proof))?;
            }
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            _ => {
                trace!("Ignoring BitVMX event: {event:?}");
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::AllNoncesReady(data)
            | RskPegManagerEvents::AllSignaturesReady(data) => {
                debug!("Handling signature event {data:?}");
                for (flow_id, sig_flow) in &mut self.signature_flows {
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
            warn!("Removing pending RSK event: {event:?}");

            // properly clean up the observer before removing the event
            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}");
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

            confirmable_event.start_confirming(block_num).context("Starting confirming")?;

            self.events_confirming.insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting for confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        // Schedule pending timeouts for flows that received pegout_accepted
        self.schedule_pending_timeouts(block);

        // Check for expired timeouts
        self.handle_advance_funds_timeout_expired(block)?;

        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_transaction_status_tick()?;
        self.process_block_confirmations(block)?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PegoutFlowProcessor");
        self.pegout_flows.clear();
        self.signature_flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.tx_status_scheduler.clear();
        self.advance_funds_timeout_scheduler.clear();
        self.flows_pending_timeout.clear();
    }
}
