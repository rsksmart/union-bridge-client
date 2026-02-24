use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, OperatorChallengeResult, OutgoingBitVMXApiMessages, ReimbursementResult,
    TransactionStatus, VariableTypes,
};
use common::runtime_sync::RuntimeSync;
use common::types::RskBlockAndUncles;
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::config::AdvanceFundsConfig;
use crate::event_processor::EventProcessor;
use crate::flows::common::GlobalContext;
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::operator_take::operator_take_flow::{
    AdvanceFundsFlow, OperatorTakeTriggerData, StepData, Steps,
};
use crate::types::{
    EventStatus, OperatorTakeTriggeredEvent, PegoutRegisteredEvent, RskPegManagerEvents,
    TickScheduler,
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

pub struct AdvanceFundsFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: common::msg_broker::broker::BitVmxBrokerClientApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    global_context: GlobalContext,
    flows: HashMap<Uuid, AdvanceFundsFlow<CG, BC>>,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<Uuid>,
    // For retry logic when native bridge lacks confirmations for register_operator_take
    unconfirmed_register_operator_take: HashMap<Uuid, i16>,
    register_operator_take_retry_scheduler: TickScheduler<Uuid>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    config: AdvanceFundsConfig,
    required_confirmations: u32,
}

impl<CG, BC> AdvanceFundsFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: common::msg_broker::broker::BitVmxBrokerClientApi,
{
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        config: AdvanceFundsConfig,
        required_confirmations: u32,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            tx_status_scheduler: TickScheduler::new(),
            unconfirmed_register_operator_take: HashMap::new(),
            register_operator_take_retry_scheduler: TickScheduler::new(),
            native_bridge_verifier,
            config,
            required_confirmations,
        }
    }

    // It is not needed to generate a specific UUID for the advance funds flow, but it is useful to have a consistent way to identify the flow.
    pub fn get_advance_funds_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
        let mut hasher = Sha256::new();
        hasher.update(committee_id.as_bytes());
        hasher.update(slot_index.to_be_bytes());
        hasher.update("advance_funds");

        let hash = hasher.finalize();
        let slice = hash
            .as_slice()
            .get(..16)
            .ok_or_else(|| anyhow!("SHA256 hash too short for UUID generation"))?;
        let uuid_bytes: [u8; 16] =
            slice.try_into().context("Failed to convert hash slice to UUID bytes")?;
        Ok(Uuid::from_bytes(uuid_bytes))
    }

    fn create_flow_for_operator_take_triggered(
        &mut self,
        event: &OperatorTakeTriggeredEvent,
    ) -> Result<()> {
        let trigger_data = OperatorTakeTriggerData::try_from_event(event)?;
        let committee_id = trigger_data.committee_id;

        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping OperatorTakeTriggered for committee {committee_id} - not a member",);
            return Ok(());
        }

        let committee_uuid = Uuid::from_u128(*committee_id);
        let flow_id = Self::get_advance_funds_pid(committee_uuid, trigger_data.slot_index)?;

        if self.flows.contains_key(&flow_id) {
            debug!(
                "Advance funds flow {flow_id} already exists for committee {committee_id}, updating trigger data",
            );
        } else {
            debug!(
                "Creating advance funds flow {} for committee {} and slot {}",
                flow_id, committee_id, trigger_data.slot_index
            );
        }

        let mut flow = AdvanceFundsFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            flow_id,
            event,
            self.native_bridge_verifier.clone(),
        )?;

        flow.complete_step(StepData::OperatorTakeTriggered)?;
        self.flows.insert(flow_id, flow);

        Ok(())
    }

    fn handle_pegout_registered(&mut self, event: &PegoutRegisteredEvent) -> Result<()> {
        let pegout_registered = event.inner.clone();
        let event_committee_id = pegout_registered.committeeId;
        let event_slot_id = pegout_registered.slotId;

        if let Some(flow) = self.flows.values_mut().find(|flow| {
            let trigger = flow.trigger_data();
            *trigger.committee_id == event_committee_id && trigger.slot_id == event_slot_id
        }) {
            flow.complete_step(StepData::OperatorTakeRegistered(pegout_registered))?;
        } else {
            trace!(
                "No advance funds flow found for PegoutRegistered with committee_id {event_committee_id} slot_id {event_slot_id}",
            );
        }

        Ok(())
    }

    /// Check if there's an active flow for the given `committee_id` and `slot_id`.
    fn has_flow_for_pegout_registered(&self, committee_id: u128, slot_id: u64) -> bool {
        self.flows.values().any(|flow| {
            let trigger = flow.trigger_data();
            *trigger.committee_id == committee_id && trigger.slot_id == slot_id
        })
    }

    fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> =
            self.flows.iter().filter(|(_, flow)| flow.is_done()).map(|(k, _)| *k).collect();

        for flow_id in completed {
            debug!("Removing completed advance funds flow {flow_id}");
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
            self.flows.remove(&flow_id);
        }
    }

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::OperatorTakeTriggered(op_take) => {
                info!(
                    "Processing confirmed OperatorTakeTriggered event: flow tx {:?}",
                    op_take.tx_hash
                );
                self.create_flow_for_operator_take_triggered(op_take)?;
            }
            RskPegManagerEvents::PegoutRegistered(pegout_registered) => {
                self.handle_pegout_registered(pegout_registered)?;
            }
            _ => {
                trace!("AdvanceFundsFlowProcessor ignoring confirmed event {event:?}",);
            }
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    fn build_operator_take_triggered_event_info(
        event: &OperatorTakeTriggeredEvent,
    ) -> (String, EventStatus, common::types::BlockNumber, RskPegManagerEvents) {
        (
            format!("operator-take-triggered-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::OperatorTakeTriggered(event.clone()),
        )
    }

    fn process_block_confirmations(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            return Ok(());
        }

        self.blockchain_view.update(block);

        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter(|(_, event)| event.is_confirmed())
            .map(|(key, _)| key.clone())
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("Advance funds RSK event confirmed, removing pending {key}",);
                trace!("Advance funds event data: {:?}", event.get_data());
                if let Err(e) = event.stop_confirming() {
                    warn!("Failed to stop confirming advance funds event {key}: {e}",);
                }
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No advance funds events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    fn handle_transaction_status(
        &mut self,
        program_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        // Find the flow whose pegin_program_id matches the program_id
        let Some((flow_id, flow)) = self.flows.iter_mut().find_map(|(flow_id, flow)| {
            (flow.state.pegin_program_id == Some(*program_id)).then_some((*flow_id, flow))
        }) else {
            trace!(
                "Ignoring transaction status for program {program_id} - no matching advance funds flow with this pegin_program_id",
            );
            return Ok(());
        };

        if flow.current_step() != Steps::RequestOperatorTakeTx {
            warn!(
                "Advance funds flow {} received transaction status at unexpected step {:?}",
                flow_id,
                flow.current_step()
            );
            return Ok(());
        }

        if tx_status.confirmations >= self.config.spv_proof_min_confirmations {
            debug!(
                "Operator take transaction confirmed for flow {} with {} confirmations",
                flow_id, tx_status.confirmations
            );
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
            flow.complete_step(StepData::RequestOperatorTakeTx(tx_status))?;
        } else {
            let min_conf = self.config.spv_proof_min_confirmations;
            debug!(
                "Operator take transaction for flow {} has {} confirmations (requires {})",
                flow_id, tx_status.confirmations, min_conf
            );
            self.tx_status_scheduler.schedule(flow_id, self.config.blocks_delay_for_tx_check);
        }

        Ok(())
    }

    fn handle_transaction_status_tick(&mut self) -> Result<()> {
        if self.tx_status_scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.tx_status_scheduler.tick();
        for flow_id in ready {
            match self.flows.get_mut(&flow_id) {
                Some(flow) => {
                    debug!("Handling transaction status tick for flow {flow_id}");
                    if flow.current_step() == Steps::RequestOperatorTakeTx {
                        flow.request_transaction_status()?;
                    } else {
                        warn!(
                            "Mismatch current step for flow {} expected {:?} having {:?}",
                            flow_id,
                            Steps::RequestOperatorTakeTx,
                            flow.current_step()
                        );
                    }
                }
                None => {
                    warn!("Skipping delayed transaction status request for unknown flow {flow_id}",);
                }
            }
        }

        Ok(())
    }

    fn handle_spv_proof(&mut self, tx_id: &bitcoin::Txid, spv_proof: BtcTxSPVProof) -> Result<()> {
        let Some(flow_id) = self.flows.iter().find_map(|(flow_id, flow)| {
            (flow.operator_take_tx_id() == Some(*tx_id)).then_some(*flow_id)
        }) else {
            trace!("Ignoring SPV proof for tx {tx_id}: no matching flow");
            return Ok(());
        };

        let current_step = self
            .flows
            .get(&flow_id)
            .map(AdvanceFundsFlow::current_step)
            .ok_or_else(|| anyhow!("Flow not found for flow_id {flow_id}"))?;

        if current_step != Steps::RequestOperatorTakeSpvProof {
            warn!(
                "Advance funds flow {flow_id} received SPV proof at unexpected step {current_step:?}"
            );
            return Ok(());
        }

        // Complete the step - this will transition to RegisterOperatorTake
        // which calls invoke_contract_safe to verify Native Bridge confirmations
        let flow = self
            .flows
            .get_mut(&flow_id)
            .ok_or_else(|| anyhow!("Flow not found for flow_id {flow_id}"))?;

        if let Err(err) = flow.complete_step(StepData::SpvProof(spv_proof)) {
            if is_missing_native_bridge_confirmations(&err) {
                let attempt = self
                    .unconfirmed_register_operator_take
                    .get(&flow_id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                self.schedule_register_operator_take_retry(
                    flow_id,
                    attempt,
                    "Missing confirmations on native bridge, scheduling retry",
                );
                return Ok(());
            }
            return Err(err);
        }
        Ok(())
    }

    fn schedule_register_operator_take_retry(&mut self, flow_id: Uuid, attempt: i16, reason: &str) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_register_operator_take.insert(flow_id, attempt);
        self.register_operator_take_retry_scheduler
            .schedule(flow_id, self.config.blocks_delay_for_tx_check);
    }

    fn handle_register_operator_take_retry_tick(&mut self) {
        if self.register_operator_take_retry_scheduler.is_empty() {
            return;
        }

        for flow_id in self.register_operator_take_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_register_operator_take.remove(&flow_id) else {
                warn!("No register_operator_take retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.flows.get_mut(&flow_id) else {
                warn!("No advance funds flow found for register_operator_take retry: {flow_id}");
                continue;
            };

            if flow.current_step() != Steps::RegisterOperatorTake {
                debug!(
                    "Skipping register_operator_take retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(StepData::RetryRegisterOperatorTake) else {
                info!("Register operator take succeeded on retry for flow {flow_id}");
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for register_operator_take: {err:?}");
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_register_operator_take_retry(
                flow_id,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }
    }

    fn handle_reimbursement_result(
        &mut self,
        pegin_program_id: &Uuid,
        result: &ReimbursementResult,
    ) -> Result<()> {
        info!(
            "Received reimbursement result from pegin program_id: {} - committee_id: {}, slot_index: {}, txid: {}, challenge_result: {:?}",
            pegin_program_id,
            result.committee_id,
            result.slot_index,
            result.txid,
            result.challenge_result
        );

        let flow_id: Uuid = Self::get_advance_funds_pid(result.committee_id, result.slot_index)?;

        if let Some(flow) = self.flows.get_mut(&flow_id) {
            debug!(
                "Delivering reimbursement result to flow {}: challenge_result = {:?}",
                flow_id, result.challenge_result
            );

            if flow.committee_id_uuid() != Some(result.committee_id) {
                error!(
                    "Mismatched committee_id in reimbursement result for flow {}: stored {:?}, got {}",
                    flow_id,
                    flow.committee_id_uuid(),
                    result.committee_id
                );
                return Ok(());
            }
            // Store the pegin program_id in the flow state

            match result.challenge_result {
                OperatorChallengeResult::OperatorTake => {
                    info!(
                        "Operator take challenge succeeded for flow {} (committee: {}, slot: {})",
                        flow_id, result.committee_id, result.slot_index
                    );
                    flow.state.pegin_program_id = Some(*pegin_program_id);

                    // Continue the flow
                    flow.complete_step(StepData::ReimbursementResult(result.clone()))?;
                }
                OperatorChallengeResult::OperatorWon => {
                    // Operator won the challenge - this means we cannot proceed with operator take
                    error!(
                        "Operator won the challenge for flow {} (committee: {}, slot: {}). Cannot proceed with operator take. Terminating flow.",
                        flow_id, result.committee_id, result.slot_index
                    );
                    // Move flow to Done step
                    flow.state.step = Steps::Done;
                }
            }
        } else {
            trace!(
                "Ignoring ReimbursementResult for program_id {} (committee: {}, slot: {}.)",
                flow_id, result.committee_id, result.slot_index
            );
        }

        Ok(())
    }
}

impl<CG, BC> EventProcessor for AdvanceFundsFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: common::msg_broker::broker::BitVmxBrokerClientApi,
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                debug!(
                    "Advance funds flow processor received SetupCompleted for program_id: {program_id}",
                );
                let mut matched_flow = false;
                for (flow_id, flow) in &mut self.flows {
                    if flow_id == program_id {
                        matched_flow = true;
                        if flow.current_step() == Steps::SetupAdvanceFundsProtocol {
                            flow.complete_step(StepData::SetupCompleted)?;
                        } else {
                            debug!(
                                "Advance funds flow {} received SetupCompleted at step {:?}",
                                flow_id,
                                flow.current_step()
                            );
                        }
                    }
                }

                if !matched_flow {
                    trace!(
                        "Ignoring SetupCompleted for program {program_id}: no matching advance funds flow",
                    );
                }
            }
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                for (flow_id, flow) in &mut self.flows {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Advance funds flow {flow_id} received comm info");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            OutgoingBitVMXApiMessages::Transaction(program_id, tx_status, _) => {
                trace!(
                    "Advance funds flow processor received Transaction for program_id: {} - txid: {}",
                    program_id, tx_status.tx_id
                );
                self.handle_transaction_status(program_id, tx_status.clone())?;
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, Some(spv_proof)) => {
                self.handle_spv_proof(tx_id, spv_proof.clone())?;
            }
            OutgoingBitVMXApiMessages::SPVProof(tx_id, None) => {
                warn!(
                    "Received SPV proof event for tx {tx_id} without proof data in advance funds flow",
                );
            }
            OutgoingBitVMXApiMessages::Variable(program_id, var_name, var_value) => {
                if var_name == &ReimbursementResult::name() {
                    if let VariableTypes::String(json_str) = var_value {
                        debug!(
                            "Advance funds flow processor received reimbursement_result variable from pegin_flow_id: {program_id}",
                        );
                        let result: ReimbursementResult = serde_json::from_str(json_str)?;
                        self.handle_reimbursement_result(program_id, &result)?;
                    } else {
                        warn!("Received reimbursement_result with unexpected type: {var_value:?}",);
                    }
                } else {
                    trace!("AdvanceFundsFlowProcessor ignoring Variable with name: {var_name}",);
                }
            }
            _ => {
                trace!("AdvanceFundsFlowProcessor ignoring BitVMX event {event:?}",);
            }
        }
        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        if self.required_confirmations == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::OperatorTakeTriggered(e) => {
                Self::build_operator_take_triggered_event_info(e)
            }
            RskPegManagerEvents::PegoutRegistered(e) => {
                // Only process PegoutRegistered events that have a matching flow (by committee_id + slot_id).
                // This filters out events from the regular pegout flow (handled by pegout_processor).
                let event_committee_id = e.inner.committeeId;
                let event_slot_id = e.inner.slotId;
                if !self.has_flow_for_pegout_registered(event_committee_id, event_slot_id) {
                    trace!(
                        "AdvanceFundsFlowProcessor ignoring PegoutRegistered event for committee_id {event_committee_id} slot_id {event_slot_id} - no matching flow",
                    );
                    return Ok(());
                }
                (
                    format!("advance-funds-pegout-registered-{}", e.tx_hash),
                    e.removed,
                    e.block_number,
                    RskPegManagerEvents::PegoutRegistered(e.clone()),
                )
            }
            _ => {
                trace!("AdvanceFundsFlowProcessor ignoring RSK event {event:?}");
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending advance funds event {event:?}");
            if let Some(mut removed_event) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_event.stop_confirming() {
                    warn!("Failed to stop confirming removed advance funds event {id}: {e}",);
                }
            } else {
                warn!("Tried to remove non-existing pending advance funds event with id {id}",);
            }
        } else {
            debug!(
                "Adding pending advance funds event {event:?}, start confirming at block {block_num}",
            );

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                self.required_confirmations,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event
                .start_confirming(block_num)
                .context("Starting confirming advance funds event")?;

            self.events_confirming.insert(confirmable_event.id(), confirmable_event);
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.process_block_confirmations(block)?;
        self.handle_transaction_status_tick()?;
        self.handle_register_operator_take_retry_tick();
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down AdvanceFundsFlowProcessor");
        self.flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.tx_status_scheduler.clear();
        self.register_operator_take_retry_scheduler.clear();
        self.unconfirmed_register_operator_take.clear();
    }
}
