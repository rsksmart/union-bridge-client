use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use common::msg_broker::bitvmx_types::{
    AdvanceFundsRegistered, FundsAdvanceSPV, OutgoingBitVMXApiMessages, UnionSPVNotification,
    UnionTxType, VariableTypes,
};
use common::runtime_sync::RuntimeSync;
use common::types::{Hash256, RskBlockAndUncles, TxIdParser};
use log::{debug, error, info, trace, warn};
use primitive_types::H256;
use sha2::{Digest, Sha256};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
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
    required_confirmations: u32,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    // For retry logic when native bridge lacks confirmations
    unconfirmed_register_advance_funds: HashMap<Uuid, i16>,
    register_advance_funds_retry_scheduler: TickScheduler<Uuid>,
    unconfirmed_register_reimbursement_kickoff: HashMap<Uuid, i16>,
    register_reimbursement_kickoff_retry_scheduler: TickScheduler<Uuid>,
    unconfirmed_register_operator_take: HashMap<Uuid, i16>,
    register_operator_take_retry_scheduler: TickScheduler<Uuid>,
    config: AdvanceFundsConfig,
}

impl<CG, BC> AdvanceFundsFlowProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: common::msg_broker::broker::BitVmxBrokerClientApi,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        required_confirmations: u32,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        config: AdvanceFundsConfig,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            required_confirmations,
            native_bridge_verifier,
            unconfirmed_register_advance_funds: HashMap::new(),
            register_advance_funds_retry_scheduler: TickScheduler::new(),
            unconfirmed_register_reimbursement_kickoff: HashMap::new(),
            register_reimbursement_kickoff_retry_scheduler: TickScheduler::new(),
            unconfirmed_register_operator_take: HashMap::new(),
            register_operator_take_retry_scheduler: TickScheduler::new(),
            config,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        contracts_gateway: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync: RuntimeSync::new().expect("Failed to create runtime sync for test processor"),
            bitvmx_broker,
            global_context,
            flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            required_confirmations: 5,
            native_bridge_verifier: NativeBridgeVerifier::Dummy,
            unconfirmed_register_advance_funds: HashMap::new(),
            register_advance_funds_retry_scheduler: TickScheduler::new(),
            unconfirmed_register_reimbursement_kickoff: HashMap::new(),
            register_reimbursement_kickoff_retry_scheduler: TickScheduler::new(),
            unconfirmed_register_operator_take: HashMap::new(),
            register_operator_take_retry_scheduler: TickScheduler::new(),
            config: AdvanceFundsConfig::default(),
        }
    }

    fn schedule_register_advance_funds_retry(&mut self, flow_id: Uuid, attempt: i16, reason: &str) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_register_advance_funds.insert(flow_id, attempt);
        self.register_advance_funds_retry_scheduler
            .schedule(flow_id, self.config.blocks_delay_for_tx_check);
    }

    fn handle_register_advance_funds_retry_tick(&mut self) {
        if self.register_advance_funds_retry_scheduler.is_empty() {
            return;
        }

        for flow_id in self.register_advance_funds_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_register_advance_funds.remove(&flow_id) else {
                warn!("No register_advance_funds retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.flows.get_mut(&flow_id) else {
                warn!("No advance funds flow found for register_advance_funds retry: {flow_id}");
                continue;
            };

            if flow.current_step() != Steps::RegisterAdvanceFunds {
                debug!(
                    "Skipping register_advance_funds retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(StepData::RetryRegisterAdvanceFunds) else {
                info!("Register advance funds succeeded on retry for flow {flow_id}");
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for register_advance_funds: {err:?}");
                continue;
            }

            self.schedule_register_advance_funds_retry(
                flow_id,
                attempt.saturating_add(1),
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }
    }

    fn schedule_register_reimbursement_kickoff_retry(
        &mut self,
        flow_id: Uuid,
        attempt: i16,
        reason: &str,
    ) {
        info!("{reason} for flow {flow_id} (attempt {attempt})");
        self.unconfirmed_register_reimbursement_kickoff.insert(flow_id, attempt);
        self.register_reimbursement_kickoff_retry_scheduler
            .schedule(flow_id, self.config.blocks_delay_for_tx_check);
    }

    fn handle_register_reimbursement_kickoff_retry_tick(&mut self) {
        if self.register_reimbursement_kickoff_retry_scheduler.is_empty() {
            return;
        }

        for flow_id in self.register_reimbursement_kickoff_retry_scheduler.tick() {
            let Some(attempt) = self.unconfirmed_register_reimbursement_kickoff.remove(&flow_id)
            else {
                warn!("No register_reimbursement_kickoff retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.flows.get_mut(&flow_id) else {
                warn!(
                    "No advance funds flow found for register_reimbursement_kickoff retry: {flow_id}"
                );
                continue;
            };

            if flow.current_step() != Steps::RegisterReimbursementKickoff {
                debug!(
                    "Skipping register_reimbursement_kickoff retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(StepData::RetryRegisterReimbursementKickoff) else {
                info!("Register reimbursement kickoff succeeded on retry for flow {flow_id}");
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for register_reimbursement_kickoff: {err:?}");
                continue;
            }

            self.schedule_register_reimbursement_kickoff_retry(
                flow_id,
                attempt.saturating_add(1),
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }
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
            self.native_bridge_verifier.clone(),
            flow_id,
            event,
        )?;

        flow.complete_step(StepData::OperatorTakeTriggered)?;
        self.flows.insert(flow_id, flow);

        Ok(())
    }

    fn handle_pegout_registered(&mut self, event: &PegoutRegisteredEvent) -> Result<()> {
        let pegout_registered = event.inner.clone();
        let event_committee_id = pegout_registered.committeeId;
        let event_slot_id = pegout_registered.streamInfo.slotId;

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

    fn complete_flow_by_pegout_id(
        &mut self,
        pegout_id: Hash256,
        expected_step: Steps,
        step_data: StepData,
        event_name: &str,
    ) -> Result<()> {
        if let Some(flow) =
            self.flows.values_mut().find(|f| f.trigger_data().pegout_id == pegout_id)
        {
            if flow.current_step() == expected_step {
                info!("{event_name} confirmed for pegout_id {pegout_id}");
                flow.complete_step(step_data)?;
            } else {
                warn!(
                    "Received {event_name} but flow is at {:?}, expected {expected_step:?}",
                    flow.current_step()
                );
            }
        } else {
            trace!("No flow found for {event_name} with pegout_id {pegout_id}");
        }
        Ok(())
    }

    fn has_flow_for_pegout_id(&self, pegout_id: Hash256) -> bool {
        self.flows.values().any(|flow| flow.trigger_data().pegout_id == pegout_id)
    }

    /// Check if there's an active flow for the given `committee_id` and `slot_id`.
    fn has_flow_for_pegout_registered(&self, committee_id: u128, slot_id: u64) -> bool {
        self.flows.values().any(|flow| {
            let trigger = flow.trigger_data();
            *trigger.committee_id == committee_id && trigger.slot_id == slot_id
        })
    }

    fn build_advance_funds_registered(
        event: &union_contracts::bindings::pegout_manager::PegoutManager::AdvanceFundsRegistered,
    ) -> Result<AdvanceFundsRegistered> {
        let committee_id = Uuid::from_u128(event.committeeId);
        let slot_index = usize::try_from(event.streamInfo.slotId)
            .context("Failed to convert slotId to usize")?;
        let txid = TxIdParser::fb_32_to_txid(event.txid);
        let pegout_id = event.pegoutId.as_slice().to_vec();

        let xonly =
            bitcoin::secp256k1::XOnlyPublicKey::from_slice(event.operatorTakePubKey.as_slice())
                .context("Failed to parse operatorTakePubKey as x-only key")?;
        let operator_pubkey = bitcoin::PublicKey::new(xonly.public_key(bitcoin::key::Parity::Even));

        Ok(AdvanceFundsRegistered { committee_id, slot_index, txid, pegout_id, operator_pubkey })
    }

    fn cleanup_completed_flows(&mut self) {
        let completed: Vec<_> =
            self.flows.iter().filter(|(_, flow)| flow.is_done()).map(|(k, _)| *k).collect();

        for flow_id in completed {
            debug!("Removing completed advance funds flow {flow_id}");
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
            RskPegManagerEvents::AdvanceFundsRegistered(e) => {
                let ev = &e.inner;
                let data = Self::build_advance_funds_registered(ev)?;
                self.complete_flow_by_pegout_id(
                    Hash256::from(ev.pegoutId),
                    Steps::RegisterAdvanceFunds,
                    StepData::AdvanceFundsConfirmed(data),
                    "AdvanceFundsRegistered",
                )?;
            }
            RskPegManagerEvents::ReimbursementKickoffRegistered(e) => {
                self.complete_flow_by_pegout_id(
                    Hash256::from(e.inner.pegoutId),
                    Steps::RegisterReimbursementKickoff,
                    StepData::ReimbursementKickoffConfirmed,
                    "ReimbursementKickoffRegistered",
                )?;
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

    fn stop_confirming_event(&mut self, id: &str) -> Option<ConfirmableEventWithData> {
        let mut event = self.events_confirming.remove(id)?;
        if let Err(e) = event.stop_confirming() {
            warn!("Failed to stop confirming advance funds event {id}: {e}");
        }
        if self.events_confirming.is_empty() {
            self.blockchain_view.clear();
        }
        Some(event)
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
            if let Some(event) = self.stop_confirming_event(&key) {
                debug!("Advance funds RSK event confirmed, removing pending {key}",);
                trace!("Advance funds event data: {:?}", event.get_data());
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        self.cleanup_completed_flows();
        Ok(())
    }

    fn handle_advance_funds_spv(&mut self, spv_data: &FundsAdvanceSPV) -> Result<()> {
        info!(
            "Received advance funds SPV - committee_id: {}, slot_index: {}, txid: {}",
            spv_data.committee_id, spv_data.slot_index, spv_data.txid
        );

        let Ok(pegout_id_bytes) = <[u8; 32]>::try_from(spv_data.pegout_id.as_slice()) else {
            warn!(
                "Ignoring funds_advance_spv with invalid pegout_id length: expected 32 bytes, got {}",
                spv_data.pegout_id.len()
            );
            return Ok(());
        };
        let pegout_id: Hash256 = Hash256::from(H256::from(pegout_id_bytes));

        let Some((flow_id_ref, flow)) =
            self.flows.iter_mut().find(|(_, flow)| flow.trigger_data().pegout_id == pegout_id)
        else {
            trace!("Ignoring funds_advance_spv for pegout_id {pegout_id} - no matching flow");
            return Ok(());
        };

        // Copy flow_id early so we can use it after the `flow` borrow ends.
        let flow_id = *flow_id_ref;

        if flow.committee_id_uuid() != spv_data.committee_id {
            warn!(
                "Mismatched committee_id in funds_advance_spv for flow {}: expected {}, got {}",
                flow_id,
                flow.committee_id_uuid(),
                spv_data.committee_id
            );
        }

        let expected_slot = flow.trigger_data().slot_index;
        if expected_slot != spv_data.slot_index {
            warn!(
                "Mismatched slot_index in funds_advance_spv for flow {}: expected {}, got {}",
                flow_id, expected_slot, spv_data.slot_index
            );
        }

        // Capture whether a retry is needed; resolve before calling &mut self methods.
        let needs_retry = if flow.current_step() == Steps::WaitForAdvanceFundsSPV {
            match flow.complete_step(StepData::AdvanceFundsSPV(spv_data.clone())) {
                Ok(()) => false,
                Err(err) if is_missing_native_bridge_confirmations(&err) => true,
                Err(err) => return Err(err),
            }
        } else if flow.current_step() == Steps::SetupAdvanceFundsProtocol {
            info!(
                "Flow {} not yet at WaitForAdvanceFundsSPV (current: {:?}), buffering SPV",
                flow_id,
                flow.current_step()
            );
            flow.state.advance_funds_spv = Some(spv_data.clone());
            false
        } else {
            warn!(
                "Advance funds flow {} received funds_advance_spv at unexpected step {:?}",
                flow_id,
                flow.current_step()
            );
            false
        };
        // `flow` borrow ends here — safe to call &mut self methods below.

        if needs_retry {
            let attempt = self
                .unconfirmed_register_advance_funds
                .get(&flow_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            self.schedule_register_advance_funds_retry(
                flow_id,
                attempt,
                "Missing confirmations on native bridge, scheduling retry",
            );
        }

        Ok(())
    }

    fn handle_union_spv_notification(&mut self, notification: &UnionSPVNotification) -> Result<()> {
        match notification.tx_type {
            UnionTxType::ReimbursementKickoff => {
                self.handle_reimbursement_kickoff_spv_notification(notification)?;
            }
            UnionTxType::OperatorTake => {
                self.handle_operator_take_spv_notification(notification)?;
            }
            _ => {
                trace!(
                    "AdvanceFundsFlowProcessor ignoring UnionSPVNotification with tx_type: {:?}",
                    notification.tx_type
                );
            }
        }
        Ok(())
    }

    fn handle_reimbursement_kickoff_spv_notification(
        &mut self,
        notification: &UnionSPVNotification,
    ) -> Result<()> {
        info!(
            "Received ReimbursementKickoff SPV notification - committee_id: {}, slot_index: {}, txid: {}",
            notification.committee_id, notification.slot_index, notification.txid
        );

        let flow_id =
            Self::get_advance_funds_pid(notification.committee_id, notification.slot_index)?;

        let Some(flow) = self.flows.get_mut(&flow_id) else {
            trace!(
                "Ignoring ReimbursementKickoff SPV for committee {} slot {} - no matching flow",
                notification.committee_id, notification.slot_index
            );
            return Ok(());
        };

        let spv_proof = notification.spv_proof.clone().ok_or_else(|| {
            anyhow!("ReimbursementKickoff SPV notification missing spv_proof data")
        })?;

        let needs_retry = if flow.current_step() == Steps::WaitForReimbursementKickoffSpv {
            match flow.complete_step(StepData::ReimbursementKickoffSPV(spv_proof)) {
                Ok(()) => false,
                Err(err) if is_missing_native_bridge_confirmations(&err) => true,
                Err(err) => return Err(err),
            }
        } else if flow.current_step() == Steps::RegisterAdvanceFunds {
            info!(
                "Flow {} not yet at WaitForReimbursementKickoffSpv (current: {:?}), buffering SPV",
                flow_id,
                flow.current_step()
            );
            flow.state.reimbursement_kickoff_spv = Some(spv_proof);
            false
        } else {
            warn!(
                "Flow {} received ReimbursementKickoff SPV at unexpected step {:?}",
                flow_id,
                flow.current_step()
            );
            false
        };
        // `flow` borrow ends here — safe to call &mut self methods below.

        if needs_retry {
            let attempt = self
                .unconfirmed_register_reimbursement_kickoff
                .get(&flow_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            self.schedule_register_reimbursement_kickoff_retry(
                flow_id,
                attempt,
                "Missing confirmations on native bridge, scheduling retry",
            );
        }

        Ok(())
    }

    fn handle_operator_take_spv_notification(
        &mut self,
        notification: &UnionSPVNotification,
    ) -> Result<()> {
        info!(
            "Received OperatorTake SPV notification - committee_id: {}, slot_index: {}, txid: {}",
            notification.committee_id, notification.slot_index, notification.txid
        );

        let flow_id =
            Self::get_advance_funds_pid(notification.committee_id, notification.slot_index)?;

        let Some(flow) = self.flows.get_mut(&flow_id) else {
            debug!(
                "Ignoring OperatorTake SPV for committee {} slot {} - no matching flow",
                notification.committee_id, notification.slot_index
            );
            return Ok(());
        };

        let spv_proof = notification
            .spv_proof
            .clone()
            .ok_or_else(|| anyhow!("OperatorTake SPV notification missing spv_proof data"))?;

        let needs_retry = if flow.current_step() == Steps::WaitForOperatorTakeSpv {
            match flow.complete_step(StepData::OperatorTakeSPV(spv_proof)) {
                Ok(()) => false,
                Err(err) if is_missing_native_bridge_confirmations(&err) => true,
                Err(err) => return Err(err),
            }
        } else {
            info!(
                "Flow {} not yet at WaitForOperatorTakeSpv (current: {:?}), buffering SPV",
                flow_id,
                flow.current_step()
            );
            flow.state.operator_take_spv = Some(spv_proof);
            false
        };
        // `flow` borrow ends here — safe to call &mut self methods below.

        if needs_retry {
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
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("Received CommInfo from BitVMX req_id: {req_id}, comm_info: {comm_info:?}");
                // For any flow in GetCommInfo step, complete the step with the CommInfo
                for (flow_id, flow) in &mut self.flows {
                    if flow.current_step() == Steps::GetCommInfo {
                        debug!("Advance funds flow {flow_id} received comm info");
                        flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                    }
                }
            }
            OutgoingBitVMXApiMessages::Variable(program_id, var_name, var_value) => {
                if var_name == FundsAdvanceSPV::name() {
                    if let VariableTypes::String(json_str) = var_value {
                        debug!(
                            "Advance funds flow processor received funds_advance_spv variable from program_id: {program_id}",
                        );
                        let spv_data: FundsAdvanceSPV = serde_json::from_str(json_str)?;
                        self.handle_advance_funds_spv(&spv_data)?;
                    } else {
                        warn!("Received funds_advance_spv with unexpected type: {var_value:?}");
                    }
                } else if var_name == UnionSPVNotification::name() {
                    if let VariableTypes::String(json_str) = var_value {
                        debug!(
                            "Advance funds flow processor received union_spv_notification variable from program_id: {program_id}",
                        );
                        let notification: UnionSPVNotification = serde_json::from_str(json_str)?;
                        self.handle_union_spv_notification(&notification)?;
                    } else {
                        warn!(
                            "Received union_spv_notification with unexpected type: {var_value:?}",
                        );
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
            RskPegManagerEvents::AdvanceFundsRegistered(e) => {
                if !self.has_flow_for_pegout_id(Hash256::from(e.inner.pegoutId)) {
                    trace!(
                        "AdvanceFundsFlowProcessor ignoring AdvanceFundsRegistered - no matching flow",
                    );
                    return Ok(());
                }
                (
                    format!("advance-funds-registered-{}", e.tx_hash),
                    e.removed,
                    e.block_number,
                    RskPegManagerEvents::AdvanceFundsRegistered(e.clone()),
                )
            }
            RskPegManagerEvents::ReimbursementKickoffRegistered(e) => {
                if !self.has_flow_for_pegout_id(Hash256::from(e.inner.pegoutId)) {
                    trace!(
                        "AdvanceFundsFlowProcessor ignoring ReimbursementKickoffRegistered - no matching flow",
                    );
                    return Ok(());
                }
                (
                    format!("reimbursement-kickoff-registered-{}", e.tx_hash),
                    e.removed,
                    e.block_number,
                    RskPegManagerEvents::ReimbursementKickoffRegistered(e.clone()),
                )
            }
            RskPegManagerEvents::PegoutRegistered(e) => {
                let event_committee_id = e.inner.committeeId;
                let event_slot_id = e.inner.streamInfo.slotId;
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
            if self.stop_confirming_event(&id).is_none() {
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
        self.handle_register_advance_funds_retry_tick();
        self.handle_register_reimbursement_kickoff_retry_tick();
        self.handle_register_operator_take_retry_tick();
        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down AdvanceFundsFlowProcessor");
        self.flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.register_advance_funds_retry_scheduler.clear();
        self.unconfirmed_register_advance_funds.clear();
        self.register_reimbursement_kickoff_retry_scheduler.clear();
        self.unconfirmed_register_reimbursement_kickoff.clear();
        self.register_operator_take_retry_scheduler.clear();
        self.unconfirmed_register_operator_take.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::str::FromStr;

    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{PublicKey, Transaction};
    use common::msg_broker::bitvmx_types::{
        BtcTxSPVProof, FundsAdvanceSPV, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
        UnionSPVNotification, UnionTxType,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::types::{Address, CommitteeId, Hash256};
    use primitive_types::{H160, H256};
    use uuid::Uuid;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;

    fn test_trigger_data(committee_id: Uuid, slot_index: usize) -> OperatorTakeTriggerData {
        OperatorTakeTriggerData {
            pegout_txid: Hash256::from(H256::from_low_u64_be(11)),
            pegout_id: Hash256::from(H256::from_low_u64_be(22)),
            committee_id: CommitteeId::from(committee_id.as_u128()),
            slot_id: slot_index as u64,
            slot_index,
            user_pubkey: PublicKey::from_str(
                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid test pubkey"),
            take_operator_address: Address::from(H160::from_low_u64_be(33)),
            operator_take_pubkey: PublicKey::from_str(
                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid operator take pubkey"),
        }
    }

    fn test_spv_proof() -> BtcTxSPVProof {
        BtcTxSPVProof {
            block_hash: "00".repeat(32),
            tx: Transaction {
                version: Version(2),
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            merkle_branch_path: "0".to_string(),
            merkle_branch_hashes: vec![],
        }
    }

    #[test]
    fn buffers_reimbursement_kickoff_spv_while_waiting_for_advance_funds_confirmation() {
        let committee_id = Uuid::new_v4();
        let slot_index = 3;
        let flow_id = AdvanceFundsFlowProcessor::<MockRskContractsGatewayApi, MockBitVmxBroker>::get_advance_funds_pid(
            committee_id,
            slot_index,
        )
            .expect("flow id");
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterAdvanceFunds,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        let proof = test_spv_proof();
        let notification = UnionSPVNotification {
            txid: proof.tx.compute_txid(),
            committee_id,
            slot_index,
            spv_proof: Some(proof.clone()),
            tx_type: UnionTxType::ReimbursementKickoff,
        };

        processor
            .handle_union_spv_notification(&notification)
            .expect("should buffer early reimbursement kickoff spv");

        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterAdvanceFunds);
        assert_eq!(
            flow.state
                .reimbursement_kickoff_spv
                .as_ref()
                .expect("proof should be buffered")
                .tx
                .compute_txid(),
            proof.tx.compute_txid()
        );
    }

    #[test]
    fn buffers_advance_funds_spv_until_wait_step_starts() {
        let committee_id = Uuid::new_v4();
        let slot_index = 1;
        let flow_id = Uuid::new_v4();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data.clone(),
            Steps::SetupAdvanceFundsProtocol,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        let proof = test_spv_proof();
        let spv = FundsAdvanceSPV {
            txid: proof.tx.compute_txid(),
            committee_id,
            slot_index,
            pegout_id: trigger_data.pegout_id.value().as_bytes().to_vec(),
            spv_proof: proof.clone(),
        };

        processor.handle_advance_funds_spv(&spv).expect("should buffer early advance funds spv");

        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::SetupAdvanceFundsProtocol);
        assert_eq!(
            flow.state.advance_funds_spv.as_ref().expect("proof should be buffered").txid,
            proof.tx.compute_txid()
        );
    }
}
