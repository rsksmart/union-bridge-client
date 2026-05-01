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
use crate::event_processor::EventProcessor;
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi,
};
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::common::{GlobalContext, Signaling};
use crate::flows::pegout::pegout_flow::{PegoutFlow, State, StepData, Steps};
use crate::store::{CoordinatorStoreApi, StorePrefix, cleanup_flows_matching, restore_flows};
use crate::types::{
    AdminRequest, EventStatus, FlowKind, RegisterSignaturesBitVmxData, RskPegManagerEvents,
    TickScheduler, TimeBasedScheduler, UserRequests,
};

fn is_missing_native_bridge_confirmations(err: &anyhow::Error) -> bool {
    use transaction_dispatcher::rsk_gateway::DomainErrors;
    err.chain().any(|cause| {
        if let Some(domain_err) = cause.downcast_ref::<DomainErrors>() {
            matches!(domain_err, DomainErrors::MissingConfirmationsOnNativeBridge(_))
        } else {
            false
        }
    })
}

pub const PEGOUT_ACCEPTED_NAME: &str = "pegout_accepted";

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
    // For retry logic when native bridge lacks confirmations for register_pegout
    unconfirmed_register_pegout: HashMap<Uuid, i16>,
    register_pegout_retry_scheduler: TickScheduler<Uuid>,
    store: Rc<S>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    advance_funds_timeout_secs: u64,
    required_confirmations: u32,
    btc_confirmations: u32,
    btc_status_retry_blocks: u32,
    // Runtime environment for force flags (only active when config.environment is local)
    environment: Option<String>,
    signaling: Rc<Signaling>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        advance_funds_timeout_secs: u64,
        required_confirmations: u32,
        btc_confirmations: u32,
        btc_status_retry_blocks: u32,
        environment: Option<&str>,
    ) -> Self {
        let factory = BtcSignatureSubFlowFactory::new(
            contracts_gateway.clone(),
            rt_sync.clone(),
            required_confirmations,
        );
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
            unconfirmed_register_pegout: HashMap::new(),
            register_pegout_retry_scheduler: TickScheduler::new(),
            store,
            native_bridge_verifier,
            advance_funds_timeout_secs,
            required_confirmations,
            btc_confirmations,
            btc_status_retry_blocks,
            environment: environment.map(String::from),
            signaling,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore_or_new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: &Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        advance_funds_timeout_secs: u64,
        required_confirmations: u32,
        btc_confirmations: u32,
        btc_status_retry_blocks: u32,
        environment: Option<&str>,
    ) -> Result<Self> {
        let mut processor = Self::new(
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            Rc::clone(store),
            signaling,
            native_bridge_verifier,
            advance_funds_timeout_secs,
            required_confirmations,
            btc_confirmations,
            btc_status_retry_blocks,
            environment,
        );

        let flow_factory = |saved_state: State| {
            PegoutFlow::from_saved_state(
                Rc::clone(&processor.contracts_gateway),
                processor.rt_sync.clone(),
                Rc::clone(&processor.bitvmx_broker),
                saved_state,
                Rc::clone(&processor.store),
                processor.signaling.clone(),
                processor.native_bridge_verifier.clone(),
            )
        };

        processor.pegout_flows =
            restore_flows(store.as_ref(), StorePrefix::PegoutFlow, flow_factory)?;

        Ok(processor)
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
    pub fn create_flow_for_pegout_requested(
        &mut self,
        event: &crate::types::PegoutRequestedEvent,
    ) -> Result<()> {
        let committee_id: CommitteeId = event.inner.committeeId.try_into()?;

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PegoutRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PegoutRequested event with committee id {committee_id}, as member I should respond"
        );

        let slot_index = usize::try_from(event.inner.slotId)
            .map_err(|_| anyhow!("slotId {} too large for usize", event.inner.slotId))?;
        let committee_uuid: Uuid = Uuid::from_u128(event.inner.committeeId.try_into()?);
        let flow_id = Self::get_user_take_pid(committee_uuid, slot_index)?;

        let mut flow = PegoutFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            flow_id,
            event,
            Rc::clone(&self.store),
            self.signaling.clone(),
            self.native_bridge_verifier.clone(),
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
        let pegout_registered = pr.inner.clone();
        let pegout_registered_txid: Txid = TxIdParser::fb_32_to_txid(pegout_registered.txid);
        let flow_opt = self
            .pegout_flows
            .values_mut()
            .find(|flow| flow.get_user_take_txid() == Some(pegout_registered_txid));

        if let Some(flow) = flow_opt {
            // Keep correlation strict: PegoutRegistered is matched only by user_take_txid.
            // We intentionally do not broaden matching by committee or slot, because the
            // PegoutAccepted checkpoint gives the flow the shared txid needed for safe
            // convergence on the confirmed terminal event.
            flow.complete_step(&StepData::PegoutRegistered(pr.clone()))?;
        } else {
            warn!("No matching pegout flow found for PegoutRegistered event: {pr:?}");
        }
        Ok(())
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
                self.create_flow_for_pegout_requested(pr)?;
            }
            RskPegManagerEvents::PegoutRegistered(pr) => {
                self.handle_pegout_registered(pr)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_terminal_flows();
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
            // Always remove the signature flow when it's done
            self.signature_flows.remove(flow_id);

            if let Some(flow) = self.pegout_flows.get_mut(flow_id) {
                // Only complete the step if the flow is still waiting for signatures
                if flow.current_step() != Steps::WaitUserTakeSignaturesReady {
                    warn!(
                        "Signature flow completed for flow_id: {flow_id} but flow is at step {:?}, expected {:?}. Skipping dispatch step.",
                        flow.current_step(),
                        Steps::WaitUserTakeSignaturesReady
                    );
                    continue;
                }

                flow.complete_step(&StepData::UserTakeSignaturesReady)?;

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

        if confirmations >= self.btc_confirmations {
            debug!("Transaction confirmed with sufficient confirmations for flow_id: {flow_id}");
            flow.complete_step(&StepData::TransactionConfirmed(tx_status))?;
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
        } else {
            let min_conf = self.btc_confirmations;
            debug!(
                "Bitcoin transaction {tx_id} missing confirmations ({confirmations}/{min_conf}) for flow_id {flow_id}, rescheduling"
            );
            self.tx_status_scheduler.schedule(flow_id, self.btc_status_retry_blocks);
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

    fn stop_confirming_event(&mut self, id: &str) -> Option<ConfirmableEventWithData> {
        let mut event = self.events_confirming.remove(id)?;
        if let Err(e) = event.stop_confirming() {
            error!("Failed to stop confirming for event {id}: {e}");
        }
        if self.events_confirming.is_empty() {
            self.blockchain_view.clear();
        }
        Some(event)
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
            if let Some(event) = self.stop_confirming_event(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn schedule_pending_timeouts(&mut self, block: &RskBlockAndUncles) {
        if self.flows_pending_timeout.is_empty() {
            return;
        }

        let current_timestamp = block.block().timestamp().value();
        let pending_flows: Vec<Uuid> = self.flows_pending_timeout.iter().copied().collect();

        for flow_id in pending_flows {
            if let Some(flow) = self.pegout_flows.get(&flow_id) {
                // Only schedule if flow is still waiting for signatures (not yet dispatched)
                if flow.current_step() == Steps::WaitUserTakeSignaturesReady {
                    self.advance_funds_timeout_scheduler.schedule(
                        flow_id,
                        current_timestamp,
                        self.advance_funds_timeout_secs,
                    );
                    info!(
                        "Scheduled advance funds timeout for flow_id: {} at timestamp: {} (expires at: {})",
                        flow_id,
                        current_timestamp,
                        current_timestamp + self.advance_funds_timeout_secs
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
    /// This completes the `WaitUserTakeSignaturesReady` step with `TriggerOperatorTakeTimeout` data
    fn trigger_operator_take_for_flow(&mut self, flow_id: Uuid) -> Result<()> {
        let flow = self
            .pegout_flows
            .get_mut(&flow_id)
            .ok_or_else(|| anyhow!("Flow not found for flow_id: {flow_id}"))?;

        // Verify flow is still in the expected state
        if flow.current_step() != Steps::WaitUserTakeSignaturesReady {
            warn!(
                "Cannot trigger operator take for flow_id: {} - flow is at step {:?}, expected {:?}",
                flow_id,
                flow.current_step(),
                Steps::WaitUserTakeSignaturesReady
            );
            return Ok(());
        }

        info!(
            "Timeout expired for flow_id: {flow_id}, completing WaitUserTakeSignaturesReady step with TriggerOperatorTakeTimeout",
        );

        // Remove the signature flow since we're bypassing it via timeout
        if self.signature_flows.remove(&flow_id).is_some() {
            debug!("Removed signature flow for flow_id: {flow_id} due to timeout");
        }

        // Complete the WaitUserTakeSignaturesReady step with timeout data
        // This will transition to TriggerOperatorTake step, which will call trigger_operator_take
        flow.complete_step(&StepData::TriggerOperatorTakeTimeout)?;

        // After trigger_operator_take completes in start_step, complete TriggerOperatorTake step
        // to transition to Done
        if flow.current_step() == Steps::TriggerOperatorTake {
            flow.complete_step(&StepData::TriggerOperatorTakeTimeout)?;
        }

        Ok(())
    }

    /// Mark a pegout flow as failed and stop pending local work for it.
    fn fail_flow(&mut self, flow_id: Uuid, reason: &str) -> Result<()> {
        if let Some(flow) = self.pegout_flows.get_mut(&flow_id) {
            flow.mark_failed(reason)?;
            warn!("Admin marked pegout flow {flow_id} as failed: {reason}");
        } else {
            warn!("Admin requested fail for unknown pegout flow {flow_id}: {reason}");
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn schedule_register_pegout_retry(&mut self, flow_id: Uuid, attempt: i16, reason: &str) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_register_pegout.insert(flow_id, attempt);
        self.register_pegout_retry_scheduler.schedule(flow_id, self.btc_status_retry_blocks);
    }

    fn handle_register_pegout_retry_tick(&mut self) {
        if self.register_pegout_retry_scheduler.is_empty() {
            return;
        }

        for flow_id in self.register_pegout_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_register_pegout.remove(&flow_id) else {
                warn!("No register_pegout retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.pegout_flows.get_mut(&flow_id) else {
                warn!("No pegout flow found for register_pegout retry: {flow_id}");
                continue;
            };

            if flow.current_step() != Steps::RegisterPegout {
                debug!(
                    "Skipping register_pegout retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(&StepData::RetryRegisterPegout) else {
                info!("Register pegout succeeded on retry for flow {flow_id}");
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for register_pegout: {err:?}");
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_register_pegout_retry(
                flow_id,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }
    }

    fn cleanup_terminal_flow_state(&mut self) {
        let terminal_flow_ids: Vec<_> = self
            .pegout_flows
            .iter()
            .filter(|(_, flow)| flow.is_terminal())
            .flat_map(|(map_flow_id, flow)| [*map_flow_id, flow.flow_id()])
            .collect();

        for flow_id in terminal_flow_ids {
            self.signature_flows.remove(&flow_id);
            self.tx_status_scheduler.cancel(&flow_id);
            self.advance_funds_timeout_scheduler.cancel(&flow_id);
            self.register_pegout_retry_scheduler.cancel(&flow_id);
            self.flows_pending_timeout.remove(&flow_id);
            self.unconfirmed_register_pegout.remove(&flow_id);
        }
    }

    fn cleanup_terminal_flows(&mut self) {
        self.cleanup_terminal_flow_state();
        cleanup_flows_matching(
            self.store.as_ref(),
            StorePrefix::PegoutFlow,
            &mut self.pegout_flows,
            PegoutFlow::is_terminal,
        );
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
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        self.cleanup_terminal_flows();

        // Pegout flows are created from RSK events, not user requests, except for the
        // admin "fail flow" lever — handled here so the cleanup runs in this processor's
        // single-threaded context alongside its in-memory state.
        if let UserRequests::Admin(AdminRequest::FailFlow { kind, flow_id, reason }) = req
            && *kind == FlowKind::Pegout
        {
            self.fail_flow(*flow_id, reason)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        self.cleanup_terminal_flows();

        trace!("Processing BitVMX event: {event:?}");

        match event {
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("Received CommInfo from BitVMX req_id: {req_id}, comm_info: {comm_info:?}");
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

                // FORCE_ADVANCE: If this operator's address matches the targeted address,
                // skip the signature sub-flow so signatures never complete and the
                // advance funds timeout triggers naturally.
                let skip_signatures =
                    crate::force_flags::get_force_advance_address(self.environment.as_deref())
                        .is_some_and(|force_addr| {
                            let my_addr = self.contracts_gateway.my_address();
                            let matches = format!("{my_addr}").to_lowercase()
                                == force_addr.trim().to_lowercase();
                            if matches {
                                warn!(
                                    "[FORCE_ADVANCE] Skipping signature flow for flow_id: {flow_id} - \
                                     operator {my_addr} will not sign, timeout will trigger advance funds",
                                );
                            }
                            matches
                        });

                if !skip_signatures {
                    let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(*flow_id);
                    btc_sig_subflow.start_signature_flow(*flow_id, &register_input)?;
                    self.signature_flows.insert(*flow_id, btc_sig_subflow);
                }

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

                // First pass: find flow_id and verify step (immutable borrow)
                let Some(flow_id) = self.pegout_flows.iter().find_map(|(flow_id, flow)| {
                    (flow.get_user_take_txid() == Some(*tx_id)).then_some(*flow_id)
                }) else {
                    trace!("Ignoring SPV proof for tx_id {tx_id} without matching flow");
                    return Ok(());
                };

                // Verify step before proceeding
                let current_step = self
                    .pegout_flows
                    .get(&flow_id)
                    .map(PegoutFlow::current_step)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id {flow_id}"))?;

                if current_step != Steps::RequestUserTakeSpvProof {
                    bail!(
                        "Mismatch current step for flow {} expected {:?} having {:?}",
                        flow_id,
                        Steps::RequestUserTakeSpvProof,
                        current_step
                    );
                }

                // Complete the step - this will transition to RegisterPegout
                // which calls invoke_contract_safe to verify Native Bridge confirmations
                let flow = self
                    .pegout_flows
                    .get_mut(&flow_id)
                    .ok_or_else(|| anyhow!("Flow not found for flow_id {flow_id}"))?;

                if let Err(err) = flow.complete_step(&StepData::SpvProof(spv_proof)) {
                    if is_missing_native_bridge_confirmations(&err) {
                        let attempt = self
                            .unconfirmed_register_pegout
                            .get(&flow_id)
                            .copied()
                            .unwrap_or(0)
                            .saturating_add(1);
                        self.schedule_register_pegout_retry(
                            flow_id,
                            attempt,
                            "Missing confirmations on native bridge, scheduling retry",
                        );
                        return Ok(());
                    }
                    return Err(err);
                }
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
        self.cleanup_terminal_flows();

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
        if self.required_confirmations == 0 {
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

            if self.stop_confirming_event(&id).is_none() {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                self.required_confirmations,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event.start_confirming(block_num).context("Starting confirming")?;

            self.events_confirming.insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting Rootstock confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.cleanup_terminal_flows();

        // Schedule pending timeouts for flows that received pegout_accepted
        self.schedule_pending_timeouts(block);

        // Check for expired timeouts
        self.handle_advance_funds_timeout_expired(block)?;

        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_transaction_status_tick()?;
        self.handle_register_pegout_retry_tick();
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
        self.register_pegout_retry_scheduler.clear();
        self.unconfirmed_register_pegout.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use alloy_primitives::{Bytes, FixedBytes, U256 as AlloyU256};
    use bitcoin::Txid;
    use common::msg_broker::bitvmx_types::{
        IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, PegOutAccepted,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::test_utils::rsk_block_generator::FakeBlockGenerator;
    use common::types::{BlockHash, TxHash, TxIdParser};
    use musig2::PubNonce;
    use musig2::secp::MaybeScalar;
    use primitive_types::H256;
    use union_contracts::bindings::pegout_manager::PegoutManager::{
        BitcoinSignatureData, BtcTransaction, StreamPosition,
    };

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::pegout::pegout_flow::FlowContext;
    use crate::store::MockCoordinatorStoreApi;

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;

    type TestBtcSigSubFlow = BaseBtcSignatureSubFlow<
        crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle<
            MockRskContractsGatewayApi,
        >,
    >;

    type TestBtcSigFactory =
        crate::flows::btc_signature::btc_signature_subflow::BtcSignatureSubFlowFactory<
            MockRskContractsGatewayApi,
        >;

    type TestProcessor = PegoutFlowProcessor<
        MockRskContractsGatewayApi,
        MockBitVmxBroker,
        TestBtcSigSubFlow,
        TestBtcSigFactory,
        MockCoordinatorStoreApi,
    >;

    type TestPegoutFlow =
        PegoutFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>;

    struct TestHarness {
        processor: TestProcessor,
        contracts: Rc<MockRskContractsGatewayApi>,
        broker: Rc<MockBitVmxBroker>,
        store: Rc<MockCoordinatorStoreApi>,
        rt_sync: RuntimeSync,
    }

    impl TestHarness {
        fn new() -> Self {
            let contracts = Rc::new(MockRskContractsGatewayApi::new());
            let mut broker = MockBitVmxBroker::new();
            broker.expect_send().returning(|_| Ok(true));
            let broker = Rc::new(broker);
            let mut store = MockCoordinatorStoreApi::new();
            store.expect_save_flow::<State>().returning(|_, _| Ok(()));
            store.expect_delete_flow().returning(|_| Ok(()));
            let store = Rc::new(store);
            let rt_sync = RuntimeSync::new().unwrap();

            let processor = PegoutFlowProcessor::new(
                contracts.clone(),
                rt_sync.clone(),
                broker.clone(),
                GlobalContext::new(),
                store.clone(),
                Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
                NativeBridgeVerifier::Dummy,
                600,
                5, // required_confirmations for tests
                1,
                20,
                None,
            );

            Self { processor, contracts, broker, store, rt_sync }
        }

        fn create_flow_at_done(&self, flow_id: Uuid) -> TestPegoutFlow {
            let state = State {
                flow_id,
                step: Steps::Done,
                ctx: FlowContext {
                    pegout_requested: create_fake_pegout_requested(),
                    request_pegout_tx_hash:
                        "0xfeedfacecafebeef000000000000000000000000000000000000000000000000"
                            .to_string(),
                    my_p2p_address: None,
                    committee_output: None,
                    peg_out_accepted: Some(fake_pegout_accepted(test_txid([3u8; 32]))),
                    pegout_registered: None,
                    pegout_registered_tx: None,
                    spv_proof: None,
                    transaction_status: None,
                },
            };

            PegoutFlow::from_saved_state(
                self.contracts.clone(),
                self.rt_sync.clone(),
                self.broker.clone(),
                state,
                self.store.clone(),
                std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
                NativeBridgeVerifier::Dummy,
            )
        }

        fn create_flow_at_step(&self, flow_id: Uuid, step: Steps) -> TestPegoutFlow {
            let state = State {
                flow_id,
                step,
                ctx: FlowContext {
                    pegout_requested: create_fake_pegout_requested(),
                    request_pegout_tx_hash:
                        "0xfeedfacecafebeef000000000000000000000000000000000000000000000000"
                            .to_string(),
                    my_p2p_address: None,
                    committee_output: None,
                    peg_out_accepted: Some(fake_pegout_accepted(test_txid([3u8; 32]))),
                    pegout_registered: None,
                    pegout_registered_tx: None,
                    spv_proof: None,
                    transaction_status: None,
                },
            };

            PegoutFlow::from_saved_state(
                self.contracts.clone(),
                self.rt_sync.clone(),
                self.broker.clone(),
                state,
                self.store.clone(),
                std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
                NativeBridgeVerifier::Dummy,
            )
        }

        fn create_completed_sig_flow(&self, flow_id: Uuid) -> TestBtcSigSubFlow {
            BaseBtcSignatureSubFlow::new_completed_for_test(
                &self.contracts,
                &self.rt_sync,
                flow_id,
                5, // required_confirmations for tests
            )
        }
    }

    fn create_fake_pegout_requested() -> PegoutRequested {
        PegoutRequested {
            userPubKey: Bytes::from(vec![0x03; 33]),
            committeeId: AlloyU256::from(1u64),
            pegoutSignatureData: BitcoinSignatureData {
                tx: BtcTransaction { version: 2, inputs: vec![], outputs: vec![], locktime: 0 },
                txid: FixedBytes::default(),
                signatureHash: FixedBytes::default(),
                signatureMessage: Bytes::default(),
            },
            streamId: 0,
            packetNumber: 0,
            slotId: 0,
            amount: 100_000,
        }
    }

    fn test_txid(bytes: [u8; 32]) -> Txid {
        TxIdParser::fb_32_to_txid(FixedBytes::from(bytes))
    }

    fn default_pub_nonce() -> PubNonce {
        "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93"
            .parse::<PubNonce>()
            .expect("invalid pub nonce")
    }

    fn fake_pegout_accepted(user_take_txid: Txid) -> PegOutAccepted {
        PegOutAccepted {
            committee_id: Uuid::from_u128(1),
            user_take_txid,
            user_take_sighash: vec![7u8; 32],
            user_take_nonce: default_pub_nonce(),
            user_take_signature: MaybeScalar::Zero,
        }
    }

    fn fake_pegout_registered_event(user_take_txid: Txid) -> crate::types::PegoutRegisteredEvent {
        crate::types::EventWithBlock {
            inner: PegoutRegistered {
                blockHash: FixedBytes::from([1u8; 32]),
                txid: TxIdParser::txid_to_fb_32(user_take_txid),
                acceptPeginTxid: FixedBytes::from([2u8; 32]),
                committeeId: 1,
                streamInfo: StreamPosition {
                    streamId: 0,
                    packetNumber: 0,
                    slotId: 0,
                    pegStatus: 0,
                },
            },
            block_number: BlockNumber::from(50),
            block_hash: BlockHash::from(H256::from_low_u64_be(51)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(52)),
        }
    }

    /// Regression test for the bug where a signature flow completing after
    /// the advance-funds timeout caused "Invalid state transition: Done with
    /// data `UserTakeSignaturesReady`".
    ///
    /// Timeline of the bug:
    ///   1. Pegout flow reaches `WaitUserTakeSignaturesReady`
    ///      (waiting for BTC signatures)
    ///   2. Timeout fires -> flow moves to `TriggerOperatorTake` -> Done
    ///   3. Signature flow completes (5/5 confirmations)
    ///   4. Processor tries `flow.complete_step(UserTakeSignaturesReady)` on a Done flow -> crash
    ///
    /// The fix (9b681821) adds a guard: skip the `complete_step` call if the
    /// flow is no longer at `WaitUserTakeSignaturesReady`.
    #[test]
    fn test_signature_completion_after_timeout_does_not_crash() {
        let mut harness = TestHarness::new();
        let flow_id = Uuid::new_v4();

        let pegout_flow = harness.create_flow_at_done(flow_id);
        harness.processor.pegout_flows.insert(flow_id, pegout_flow);
        let signature_flow = harness.create_completed_sig_flow(flow_id);
        harness.processor.signature_flows.insert(flow_id, signature_flow);

        let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        let block = RskBlockAndUncles::new_no_uncles(
            block_generator
                .generate_block(BlockNumber::from(100), None)
                .expect("failed to generate test block"),
        );

        let result = harness.processor.process_unhandled_confirmed_sig_flow_events(&block);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        assert_eq!(harness.processor.pegout_flows[&flow_id].current_step(), Steps::Done);
        assert!(!harness.processor.signature_flows.contains_key(&flow_id));

        // Idempotency: a second pass must also succeed and not reintroduce state
        let result = harness.processor.process_unhandled_confirmed_sig_flow_events(&block);
        assert!(result.is_ok(), "second call expected Ok, got: {:?}", result.err());
        assert!(!harness.processor.signature_flows.contains_key(&flow_id));
    }

    #[test]
    fn handle_pegout_registered_delivers_to_confirm_user_take_transaction() {
        let mut harness = TestHarness::new();
        let flow_id = Uuid::new_v4();
        harness.processor.pegout_flows.insert(
            flow_id,
            harness.create_flow_at_step(flow_id, Steps::ConfirmUserTakeTransaction),
        );
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        harness
            .processor
            .handle_pegout_registered(&event)
            .expect("confirmed event should fast-forward");

        let flow = harness.processor.pegout_flows.get(&flow_id).expect("flow exists");
        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn handle_pegout_registered_delivers_to_request_user_take_spv_proof() {
        let mut harness = TestHarness::new();
        let flow_id = Uuid::new_v4();
        harness
            .processor
            .pegout_flows
            .insert(flow_id, harness.create_flow_at_step(flow_id, Steps::RequestUserTakeSpvProof));
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        harness
            .processor
            .handle_pegout_registered(&event)
            .expect("confirmed event should fast-forward");

        let flow = harness.processor.pegout_flows.get(&flow_id).expect("flow exists");
        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn cleanup_terminal_flows_removes_pegout_flow_and_side_state() {
        let mut harness = TestHarness::new();
        let flow_id = Uuid::new_v4();
        harness.processor.pegout_flows.insert(flow_id, harness.create_flow_at_done(flow_id));
        harness
            .processor
            .signature_flows
            .insert(flow_id, harness.create_completed_sig_flow(flow_id));
        harness.processor.tx_status_scheduler.schedule(flow_id, 1);
        harness.processor.advance_funds_timeout_scheduler.schedule(flow_id, 100, 600);
        harness.processor.register_pegout_retry_scheduler.schedule(flow_id, 1);
        harness.processor.flows_pending_timeout.insert(flow_id);
        harness.processor.unconfirmed_register_pegout.insert(flow_id, 1);

        harness.processor.cleanup_terminal_flows();

        assert!(!harness.processor.pegout_flows.contains_key(&flow_id));
        assert!(!harness.processor.signature_flows.contains_key(&flow_id));
        assert!(!harness.processor.tx_status_scheduler.is_scheduled(&flow_id));
        assert!(!harness.processor.advance_funds_timeout_scheduler.is_scheduled(&flow_id));
        assert!(!harness.processor.register_pegout_retry_scheduler.is_scheduled(&flow_id));
        assert!(!harness.processor.flows_pending_timeout.contains(&flow_id));
        assert!(!harness.processor.unconfirmed_register_pegout.contains_key(&flow_id));
    }
}
