use crate::{
    blockchain_tracker::{BlockchainView, ConfirmableEventWithData},
    config::REQUIRED_CONFIRMATIONS,
    event_processor::EventProcessor,
    flows::{
        btc_signature::{
            btc_signature_lifecycle::BtcSignatureLifeCycle,
            btc_signature_subflow::{
                BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
                BtcSignatureSubFlowFactoryApi,
            },
        },
        common::GlobalContext,
        pegin::pegin_flow::{PeginFlow, StepData, Steps},
    },
    types::{
        AllOperatorTakeTxHashesAddedEvent, EventStatus, PeginAcceptedEvent, PeginRequestedEvent,
        RegisterSignaturesBitVmxData, RskPegManagerEvents, TickScheduler, UserRequests,
    },
};

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Txid;
use common::{
    msg_broker::{
        bitvmx_types::{
            IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, PeginAcceptedMessage,
            TransactionStatus, VariableTypes,
        },
        broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi},
    },
    runtime_sync::RuntimeSync,
    types::{BlockNumber, CommitteeId, RskBlockAndUncles},
};
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use std::{
    any::type_name_of_val,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use union_contracts::bindings::peg_manager::PegManager::PeginRequested;
use uuid::Uuid;

const PEGIN_ACCEPTED_INPUT_MSG: &str = "pegin_accepted";
pub const MIN_TX_CONFIRMATIONS: u32 = 1 + 1; // +1 from Contracts, +1 to give time to the Native Bridge to get up to date with Bitcoin Node
pub const BLOCKS_DELAY_FOR_TX_CHECK: u32 = 20;

/// Processor that manages multiple pegin flow state machines
pub struct PeginFlowProcessor<CG, BC, BSF, FactoryBSF>
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
    pegin_flows: HashMap<Uuid, PeginFlow<CG, BC>>,
    signature_flows: HashMap<Uuid, BSF>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<Uuid>,
    pegin_request_tracker: HashSet<Txid>,
    // For retry logic when native bridge lacks confirmations
    unconfirmed_pegin_requests:
        HashMap<String, (common::msg_broker::bitvmx_types::BtcTxSPVProof, i16)>,
    pegin_retry_scheduler: TickScheduler<String>,
}

impl<CG, BC>
    PeginFlowProcessor<
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

        // Subscribe to BitVMX pegin events
        Self::subscribe_to_bitvmx_pegin_events(&bitvmx_broker)
            .expect("Failed to subscribe to BitVMX pegin events");

        info!("Successfully subscribed to BitVMX pegin events");

        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            btc_sig_subflow_factory: factory,
            pegin_flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            signature_flows: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
            pegin_request_tracker: HashSet::new(),
            unconfirmed_pegin_requests: HashMap::new(),
            pegin_retry_scheduler: TickScheduler::new(),
        }
    }

    pub fn get_accept_pegin_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(&slot_index.to_be_bytes());
        hasher.update("accept_pegin");

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

    /// Create a new flow for a PeginRequested event
    fn create_flow_for_pegin_requested(&mut self, event: &PeginRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.try_into()?;

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PeginRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PeginRequested event with committee id {}, as member I should respond",
            committee_id
        );

        let slot_index = event.streamPosition.slotId as usize;
        let committee_uuid: Uuid = Uuid::from_u128(event.committeeId.try_into()?);
        let flow_id = Self::get_accept_pegin_pid(committee_uuid, slot_index)?;

        let mut flow = PeginFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            flow_id,
            event.clone(),
        );

        // Initialize the flow with the PeginRequested event
        flow.complete_step(StepData::PeginRequested)?;

        self.pegin_flows.insert(flow_id, flow);

        info!(
            "Created new pegin flow {} for committee {}",
            flow_id, committee_id
        );
        Ok(())
    }

    /// Handle confirmed PeginAccepted event
    fn handle_pegin_accepted(&mut self, pa: &PeginAcceptedEvent) -> Result<()> {
        info!("Processing confirmed PeginAccepted event: {:?}", pa);

        // Find the flow corresponding to this pegin acceptance using accept_pegin_tx_hash
        let flow_opt = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid()
                .map(|txid| common::types::TxIdParser::txid_to_fb_32(txid))
                == Some(pa.inner.acceptPeginTxHash)
        });

        if let Some(flow) = flow_opt {
            flow.complete_step(StepData::PeginAccepted(pa.inner.clone()))?;
        } else {
            warn!(
                "No matching pegin flow found for PeginAccepted event: {:?}",
                pa
            );
        }
        Ok(())
    }

    /// Clean up completed flows
    pub fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> = self
            .pegin_flows
            .iter()
            .filter(|(_, flow)| flow.is_done())
            .map(|(k, _)| *k)
            .collect();

        for internal_id in completed {
            debug!("Removing completed flow: {internal_id}");
            self.pegin_flows.remove(&internal_id);
        }
    }

    /// Process confirmed RSK events
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        info!("Processing confirmed RSK event: {:?}", event);

        match event {
            RskPegManagerEvents::PeginRequested(pr) => {
                let committee_id = pr.inner.committeeId.try_into()?;
                if !self.global_context.my_committees().im_member(&committee_id) {
                    debug!(
                        "Handling PeginRequested event with committee id {}, I am NOT member so I skip",
                        committee_id
                    );
                    return Ok(());
                }
                info!("Processing confirmed PeginRequested event: {:?}", pr);
                self.create_flow_for_pegin_requested(&pr.inner)?;
            }
            RskPegManagerEvents::PeginAccepted(pa) => {
                self.handle_pegin_accepted(pa)?;
            }
            RskPegManagerEvents::AllOperatorTakeTxHashesAdded(aottah) => {
                self.handle_all_operator_take_tx_hashes_added(aottah)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    fn handle_all_operator_take_tx_hashes_added(
        &mut self,
        event: &AllOperatorTakeTxHashesAddedEvent,
    ) -> Result<()> {
        debug!(
            "Processing AllOperatorTakeTxHashesAdded: acceptPeginTxHash={}",
            event.inner.acceptPeginTxHash
        );

        // Find the flow by accept_pegin_tx_hash
        let flow_opt = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid()
                .map(|txid| common::types::TxIdParser::txid_to_fb_32(txid))
                == Some(event.inner.acceptPeginTxHash)
        });

        if let Some(flow) = flow_opt {
            let flow_id = flow.flow_id();

            // Start the BTC signature flow if not already started
            if !self.signature_flows.contains_key(&flow_id) {
                info!("Starting BTC signature flow: flow_id={}", flow_id);

                let pegin_accepted =
                    flow.state.bitvmx_pegin_accepted.as_ref().ok_or_else(|| {
                        anyhow!("PeginAcceptedMessage not found for flow_id: {}.", flow_id)
                    })?;

                let register_input =
                    RegisterSignaturesBitVmxData::try_from(pegin_accepted.clone())?;

                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(flow_id);
                btc_sig_subflow.start_signature_flow(flow_id, &register_input)?;

                self.signature_flows.insert(flow_id, btc_sig_subflow);

                // Complete the step to move to the next state
                flow.complete_step(StepData::OperatorTakeHashAdded)?;
            } else {
                error!("BTC signature flow already started: flow_id={}", flow_id);
            }
        } else {
            debug!(
                "Received AllOperatorTakeTxHashesAdded: unknown_acceptPeginTxHash={:?}",
                event.inner.acceptPeginTxHash
            );
        }

        Ok(())
    }

    /// Build event info for PeginRequested events
    fn build_pegin_requested_event_info(
        event: &PeginRequestedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegin-requested-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PeginRequested(event.clone()),
        )
    }

    fn build_pegin_accepted_event_info(
        event: &PeginAcceptedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("pegin-accepted-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PeginAccepted(event.clone()),
        )
    }

    fn build_all_operator_take_tx_hashes_added_event_info(
        event: &AllOperatorTakeTxHashesAddedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("all-operator-take-tx-hashes-added-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::AllOperatorTakeTxHashesAdded(event.clone()),
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
            if let Some(flow) = self.pegin_flows.get_mut(flow_id) {
                flow.complete_step(StepData::DispatchTransaction)?;
                self.signature_flows.remove(&flow_id);
            } else {
                warn!(
                    "Signature flow done for unknown pegin flow_id: {}. Skipping dispatch step",
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
        let flow = match self.pegin_flows.get_mut(&flow_id) {
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
            .get_accept_pegin_txid()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;

        if expected_txid != tx_id {
            return Err(anyhow!(
                "Pegin state for flow_id: {} does not match received tx_id: {} from tx status message",
                flow_id,
                tx_id
            ));
        }

        if flow.current_step() != Steps::ConfirmAcceptPeginTransaction {
            return Err(anyhow!(
                "Mismatch current step for flow {} expected {:?} having {:?}",
                flow_id,
                Steps::ConfirmAcceptPeginTransaction,
                flow.current_step()
            ));
        }

        if confirmations >= MIN_TX_CONFIRMATIONS {
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
            match self.pegin_flows.get_mut(&flow_id) {
                Some(flow) => {
                    if flow.current_step() == Steps::ConfirmAcceptPeginTransaction {
                        flow.request_transaction_status()?;
                    } else {
                        warn!(
                            "Mismatch current step for flow {} expected {:?} having {:?}",
                            flow_id,
                            Steps::ConfirmAcceptPeginTransaction,
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

    fn handle_pegin_retry_tick(&mut self) -> Result<()> {
        if self.pegin_retry_scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.pegin_retry_scheduler.tick();
        for block_hash in ready {
            debug!("(Re)trying request_pegin for block {}", block_hash);

            if let Some((spv_proof, attempt)) = self.unconfirmed_pegin_requests.remove(&block_hash)
            {
                let tx_id = spv_proof.tx.compute_txid();

                // Call requestPegin contract again
                let input: transaction_dispatcher::types::RequestPeginInput =
                    spv_proof.clone().into();
                let res = self
                    .rt_sync
                    .run(async { self.contracts_gateway.request_pegin(input).await });

                match res {
                    Ok(_) => {
                        info!("Request pegin succeeded on retry for block {}", block_hash);
                        // Remove from tracking since it succeeded
                        self.pegin_request_tracker.remove(&tx_id);
                    }
                    Err(DomainErrors::MissingConfirmationsOnNativeBridge(_)) => {
                        info!(
                            "Still missing confirmations on native bridge for block {}, scheduling another retry (attempt {})",
                            block_hash,
                            attempt + 1
                        );
                        // Store for another retry with incremented attempt
                        self.unconfirmed_pegin_requests
                            .insert(block_hash.clone(), (spv_proof, attempt + 1));
                        self.pegin_retry_scheduler
                            .schedule(block_hash, BLOCKS_DELAY_FOR_TX_CHECK);
                    }
                    Err(DomainErrors::PeginAlreadyRequested(msg)) => {
                        info!("Pegin already requested on retry: {}", msg);
                        // Remove from tracking since it's already processed
                        self.pegin_request_tracker.remove(&tx_id);
                    }
                    Err(err) => {
                        error!("Error on retry for request_pegin: {:?}", err);
                        // Don't retry on other errors
                        self.pegin_request_tracker.remove(&tx_id);
                    }
                }
            } else {
                warn!(
                    "No unconfirmed pegin request found for block: {}",
                    block_hash
                );
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

    fn subscribe_to_bitvmx_pegin_events(bitvmx_broker: &BC) -> Result<()> {
        bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SubscribeToRskPegin(),
        )?;
        Ok(())
    }

    fn handle_pegin_transaction_found(&mut self, tx_id: Txid) -> Result<()> {
        self.pegin_request_tracker.insert(tx_id);
        // When notified of a new pegin tx found, the client will immediately
        // request the SPV proof of such transaction to notify the contract
        self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::GetSPVProof(tx_id),
        )?;
        Ok(())
    }

    fn handle_spv_proof_for_request_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: common::msg_broker::bitvmx_types::BtcTxSPVProof,
    ) -> Result<()> {
        if !self.pegin_request_tracker.contains(tx_id) {
            return Ok(()); // Not a tracked pegin request
        }

        info!("Handling request pegin SPV proof: tx_id={}", tx_id);

        // Call requestPegin contract
        let input: transaction_dispatcher::types::RequestPeginInput = spv_proof.clone().into();
        let res = self
            .rt_sync
            .run(async { self.contracts_gateway.request_pegin(input).await });

        match res {
            Ok(_) => {
                // Remove from tracking set after successful processing
                self.pegin_request_tracker.remove(tx_id);
                debug!("Removed request_pegin_txid from tracking: tx_id={}", tx_id);
                Ok(())
            }
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(_)) => {
                info!(
                    "Missing confirmations on native bridge for block {}, scheduling retry",
                    spv_proof.block_hash
                );
                // Store SPV proof for retry and schedule it
                self.unconfirmed_pegin_requests.insert(
                    spv_proof.block_hash.clone(),
                    (spv_proof.clone(), 1), // Start with attempt 1
                );
                self.pegin_retry_scheduler
                    .schedule(spv_proof.block_hash, BLOCKS_DELAY_FOR_TX_CHECK);
                Ok(())
            }
            Err(DomainErrors::PeginAlreadyRequested(msg)) => {
                // This is expected if the same pegin is requested multiple times
                // We should treat it as a success case
                info!(
                    "Pegin already requested for tx_id={}, treating as expected: {}",
                    tx_id, msg
                );
                // Remove from tracking since it's already processed
                self.pegin_request_tracker.remove(tx_id);
                Ok(())
            }
            Err(domain_err) => bail!("Error executing 'requestPegin': {domain_err:?}"),
        }
    }

    fn handle_spv_proof_for_accept_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: common::msg_broker::bitvmx_types::BtcTxSPVProof,
    ) -> Result<()> {
        // Find state by matching accept_pegin_txid from bitvmx_pegin_accepted
        let flow_opt = self
            .pegin_flows
            .values_mut()
            .find(|flow| flow.get_accept_pegin_txid() == Some(*tx_id));

        if let Some(flow) = flow_opt {
            info!(
                "Handling accept pegin SPV proof: flow_id={}, tx_id={}",
                flow.flow_id(),
                tx_id
            );
            flow.complete_step(StepData::SpvProof(spv_proof))?;
        } else {
            debug!(
                "SPV proof for tx_id: {} is not related to a pegin flow",
                tx_id
            );
        }

        Ok(())
    }
}

impl<CG, BC> EventProcessor
    for PeginFlowProcessor<
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
        // Pegin flows are created from RSK events, not from user requests
        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        trace!("Processing BitVMX event: {:?}", event);

        match event {
            // Handle PeginTransactionFound from BitVMX
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, _tx_status) => {
                debug!("Received BitVMX PeginTransactionFound: tx_id={}", tx_id);
                self.handle_pegin_transaction_found(*tx_id)?;
            }
            // Handle SPVProof from BitVMX (for both request and accept pegin)
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                if let Some(spv_proof) = spv_proof_opt {
                    debug!("Received BitVMX SPVProof: tx_id={}", tx_id);
                    trace!(
                        "Received spv_proof_data for tx_id={}: {:?}",
                        tx_id, spv_proof
                    );

                    // Try to handle as request pegin first
                    self.handle_spv_proof_for_request_pegin(tx_id, spv_proof.clone())?;

                    // Then try to handle as accept pegin
                    self.handle_spv_proof_for_accept_pegin(tx_id, spv_proof.clone())?;
                } else {
                    return Err(anyhow!(
                        "Received BitVMX SPVProof event for tx_id: {}, but no SPV proof was included.",
                        tx_id
                    ));
                }
            }
            // Handle CommInfo from BitVMX
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                trace!("Received CommInfo from BitVMX: {:?}", comm_info);
                // For any flow in GetCommInfo step, complete the step with the CommInfo
                for (flow_id, flow) in self.pegin_flows.iter_mut() {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Completing GetCommInfo step for flow {flow_id}");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            // Handle PeginAccepted variable from BitVMX
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), PEGIN_ACCEPTED_INPUT_MSG) =>
            {
                info!(
                    "Received PeginAccepted variable from BitVMX for flow_id: {}",
                    flow_id
                );
                debug!("PeginAccepted data: {}", data);
                let pegin_accepted: PeginAcceptedMessage = serde_json::from_str(data)?;
                let flow = self
                    .pegin_flows
                    .get_mut(flow_id)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id: {}", flow_id))?;

                if flow.current_step() != Steps::PreparePeginSetup {
                    return Err(anyhow!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::PreparePeginSetup,
                        flow.current_step()
                    ));
                }

                flow.complete_step(StepData::BitvmxPeginAccepted(pegin_accepted))?;
            }
            // Handle SetupCompleted from BitVMX
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                if self.pegin_flows.contains_key(program_id) {
                    info!("Pegin setup was completed: flow_id={}", program_id);
                } else {
                    trace!(
                        "Ignoring BitVMX SetupCompleted for unknown program_id: {}",
                        program_id
                    );
                }
            }
            // Handle Transaction status from BitVMX
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
                // Continue with the normal flow
            }
        }

        // useful for testing purposes
        if REQUIRED_CONFIRMATIONS == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PeginRequested(e) => Self::build_pegin_requested_event_info(e),
            RskPegManagerEvents::PeginAccepted(e) => Self::build_pegin_accepted_event_info(e),
            RskPegManagerEvents::AllOperatorTakeTxHashesAdded(e) => {
                Self::build_all_operator_take_tx_hashes_added_event_info(e)
            }
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
        self.handle_pegin_retry_tick()?;
        self.process_block_confirmations(block)?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginFlowProcessor");
        self.pegin_flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.tx_status_scheduler.clear();
        self.pegin_retry_scheduler.clear();
        self.unconfirmed_pegin_requests.clear();
        self.pegin_request_tracker.clear();
        self.signature_flows.clear();
    }
}
