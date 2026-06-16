use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use common_bitvmx::bitvmx_types::{
    FundsAdvanceSPV, OPERATOR_TAKE_TX, OutgoingBitVMXApiMessages, UnionSPVNotification,
    UnionTxType, VariableTypes, advance_funds_protocol_id,
};
use common_core::types::{CommitteeId, Hash256, RskBlockAndUncles};
use common_runtime::runtime_sync::RuntimeSync;
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, info_span, trace, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::blockchain_tracker::{
    BlockchainView, ConfirmableEventWithData, ConfirmableEventWithDataSnapshot,
    restore_confirmable_events, snapshot_confirmable_events,
};
use crate::event_processor::EventProcessor;
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::common::{FlowId, GlobalContext, Signaling};
#[cfg(test)]
use crate::flows::operator_take::operator_take_flow::Steps;
use crate::flows::operator_take::operator_take_flow::{
    AdvanceFundsFlow, FlowContext, StepData, StepOutcome,
    flow_id_from_operator_take_triggered_tx_hash,
};
use crate::flows::operator_take::types::{
    OperatorTakeTriggerData, advance_funds_registered_from_event,
};
use crate::store::{CoordinatorStoreApi, StoreKey, StorePrefix, restore_flows};
use crate::types::{
    AdminRequest, EventStatus, FlowKind, OperatorTakeTriggeredEvent, PegoutRegisteredEvent,
    RetryTracker, RskPegManagerEvents, UserRequests,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdvanceFundsProcessorState {
    events_confirming: HashMap<String, ConfirmableEventWithDataSnapshot>,
    retries: RetryTracker<FlowId>,
    #[serde(default, deserialize_with = "deserialize_request_pegout_tx_hash_entries")]
    request_pegout_tx_hashes: Vec<RequestPegoutTxHashEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RequestPegoutTxHashEntry {
    committee_id: Uuid,
    slot_id: u64,
    tx_hash: String,
}

fn snapshot_request_pegout_tx_hashes(
    hashes: &HashMap<(CommitteeId, u64), String>,
) -> Vec<RequestPegoutTxHashEntry> {
    let mut entries: Vec<_> = hashes
        .iter()
        .map(|((committee_id, slot_id), tx_hash)| RequestPegoutTxHashEntry {
            committee_id: Uuid::from_u128(**committee_id),
            slot_id: *slot_id,
            tx_hash: tx_hash.clone(),
        })
        .collect();
    entries.sort_by(|left, right| {
        left.committee_id.cmp(&right.committee_id).then_with(|| left.slot_id.cmp(&right.slot_id))
    });
    entries
}

fn restore_request_pegout_tx_hashes(
    entries: Vec<RequestPegoutTxHashEntry>,
) -> HashMap<(CommitteeId, u64), String> {
    entries
        .into_iter()
        .map(|entry| {
            ((CommitteeId::from(entry.committee_id.as_u128()), entry.slot_id), entry.tx_hash)
        })
        .collect()
}

fn deserialize_request_pegout_tx_hash_entries<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<RequestPegoutTxHashEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Array(_) => {
            serde_json::from_value(value).map_err(serde::de::Error::custom)
        }
        serde_json::Value::Object(map) if map.is_empty() => Ok(Vec::new()),
        serde_json::Value::Null => Ok(Vec::new()),
        other => Err(serde::de::Error::custom(format!(
            "invalid request_pegout_tx_hashes snapshot: expected array or empty legacy map, got {other}"
        ))),
    }
}

pub(crate) struct AdvanceFundsFlowProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: common_broker::broker::BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    global_context: GlobalContext,
    flows: HashMap<FlowId, AdvanceFundsFlow<CG, BC, S>>,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    required_confirmations: u32,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    // Retry state for RSK registrations parked because the native bridge
    // lacks enough confirmations. Keyed by flow id — the flow's current step
    // determines which registration is retried (only one outstanding retry
    // per flow at any time, by construction of the state machine).
    retries: RetryTracker<FlowId>,
    btc_status_retry_blocks: u32,
    signaling: Rc<Signaling>,
    request_pegout_tx_hashes: HashMap<(CommitteeId, u64), String>,
    store: Rc<S>,
}

impl<CG, BC, S> AdvanceFundsFlowProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: common_broker::broker::BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        signaling: Rc<Signaling>,
        store: &Rc<S>,
        required_confirmations: u32,
        btc_status_retry_blocks: u32,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        let mut processor = Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            flows: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            events_confirming: HashMap::new(),
            required_confirmations,
            native_bridge_verifier,
            retries: RetryTracker::new(),
            btc_status_retry_blocks,
            signaling,
            request_pegout_tx_hashes: HashMap::new(),
            store: Rc::clone(store),
        };

        let flow_factory = |saved_state: FlowContext| {
            AdvanceFundsFlow::from_saved_state(
                Rc::clone(&processor.contracts_gateway),
                processor.rt_sync.clone(),
                Rc::clone(&processor.bitvmx_broker),
                processor.native_bridge_verifier.clone(),
                processor.signaling.clone(),
                Rc::clone(&processor.store),
                saved_state,
            )
        };

        let restored: HashMap<Uuid, AdvanceFundsFlow<CG, BC, S>> =
            restore_flows(store.as_ref(), StorePrefix::OperatorTakeFlow, flow_factory)
                .expect("Failed to load operator-take flows from store");
        processor.flows = restored.into_values().map(|flow| (flow.flow_id(), flow)).collect();

        processor
            .restore_processor_state()
            .expect("Failed to load advance funds processor state from store");
        processor.rearm_pending_retries();
        processor
    }
}

#[cfg(test)]
impl<CG, BC> AdvanceFundsFlowProcessor<CG, BC, crate::store::MockCoordinatorStoreApi>
where
    CG: RskContractsGatewayApi,
    BC: common_broker::broker::BitVmxBrokerClientApi,
{
    pub(crate) fn new_for_test(
        contracts_gateway: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
    ) -> Self {
        let mut store = crate::store::MockCoordinatorStoreApi::new();
        store.expect_save_flow::<AdvanceFundsProcessorState>().returning(|_, _| Ok(()));
        store.expect_delete_flow().returning(|_| Ok(()));
        let store = Rc::new(store);

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
            retries: RetryTracker::new(),
            btc_status_retry_blocks: 20,
            signaling: Rc::new(Signaling::new("/tmp", "disabled")),
            request_pegout_tx_hashes: HashMap::new(),
            store,
        }
    }
}

impl<CG, BC, S> AdvanceFundsFlowProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: common_broker::broker::BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn snapshot_processor_state(&self) -> AdvanceFundsProcessorState {
        AdvanceFundsProcessorState {
            events_confirming: snapshot_confirmable_events(&self.events_confirming),
            retries: self.retries.clone(),
            request_pegout_tx_hashes: snapshot_request_pegout_tx_hashes(
                &self.request_pegout_tx_hashes,
            ),
        }
    }

    fn restore_processor_state(&mut self) -> Result<()> {
        let Some(state) = self
            .store
            .load_flow::<AdvanceFundsProcessorState>(&StoreKey::AdvanceFundsProcessorState)?
        else {
            return Ok(());
        };

        self.apply_processor_state(state)
    }

    fn apply_processor_state(&mut self, state: AdvanceFundsProcessorState) -> Result<()> {
        let (blockchain_view, events_confirming) =
            restore_confirmable_events(state.events_confirming)?;
        self.events_confirming = events_confirming;
        self.blockchain_view = blockchain_view;
        self.retries = state.retries;
        self.request_pegout_tx_hashes =
            restore_request_pegout_tx_hashes(state.request_pegout_tx_hashes);
        Ok(())
    }

    fn persist_processor_state(&self) -> Result<()> {
        self.store.save_flow(&StoreKey::AdvanceFundsProcessorState, self.snapshot_processor_state())
    }

    fn rearm_pending_retries(&mut self) {
        let to_retry: Vec<FlowId> = self
            .flows
            .values()
            .filter(|flow| flow.pending_retry())
            .map(AdvanceFundsFlow::flow_id)
            .filter(|flow_id| self.retries.current_attempt(flow_id) == 0)
            .collect();

        for flow_id in to_retry {
            self.schedule_retry(flow_id, 1, "re-arming retry for restored flow");
        }
    }

    fn schedule_retry(&mut self, flow_id: FlowId, attempt: i16, reason: &str) {
        info!("Scheduling retry for flow {flow_id} (attempt {attempt}): {reason}");
        self.retries.schedule(flow_id, attempt, self.btc_status_retry_blocks);
    }

    fn handle_retry_tick(&mut self) {
        if self.retries.is_empty() {
            return;
        }

        for (flow_id, attempt) in self.retries.tick() {
            let Some(flow) = self.flows.get_mut(&flow_id) else {
                warn!("No advance funds flow found for retry: {flow_id}");
                continue;
            };
            let ancestor_pegout_id = flow.ancestor_pegout_id();
            let _span = info_span!(
                "operator_take",
                ancestor_pegout_id = %ancestor_pegout_id,
                operator_take_id = %flow_id,
            )
            .entered();

            // A retry is scheduled only when a register-step submission was
            // parked due to missing native-bridge confirmations. Re-entering
            // the same step (via `StepData::Retry`) re-runs the submission.
            // If the flow has already moved past the register step (success
            // event arrived first), the flow returns `NoOp` and we drop the
            // stale retry silently.
            match flow.complete_step(StepData::Retry) {
                Ok(StepOutcome::Done) => {
                    info!("Registration succeeded on retry for flow {flow_id}");
                }
                Ok(StepOutcome::NoOp) => {
                    debug!(
                        "Stale retry for flow {flow_id} — success event already advanced the flow",
                    );
                }
                Ok(StepOutcome::Retry { reason }) => {
                    self.schedule_retry(flow_id, attempt.saturating_add(1), &reason);
                }
                Err(err) => {
                    error!("Error on retry for flow {flow_id}: {err:?}");
                }
            }
        }
    }

    /// Find a flow by `(committee_id, slot_index)` — the keys carried by
    /// `BitVMX` `UnionSPVNotification`s. Returns the matching flow if any.
    /// Matches on the cached `bitvmx_protocol_id`, which is derived from the
    /// same `(committee_id, slot_index)` tuple at flow construction.
    ///
    /// Bails if more than one flow matches — this is an invariant violation
    /// (two flows holding the same `BitVMX` protocol id can only arise from a
    /// bug or an unhandled re-trigger scenario, and would otherwise lead to
    /// nondeterministic dispatch).
    fn flow_by_committee_slot(
        &mut self,
        committee_id: Uuid,
        slot_index: usize,
    ) -> Result<Option<&mut AdvanceFundsFlow<CG, BC, S>>> {
        let target = advance_funds_protocol_id(committee_id, slot_index);
        let mut matches =
            self.flows.values_mut().filter(|flow| flow.bitvmx_protocol_id() == target);
        let first = matches.next();
        if matches.next().is_some() {
            bail!(
                "Invariant violation: multiple advance-funds flows hold BitVMX protocol id \
                 {target} for (committee {committee_id}, slot {slot_index}); refusing to route",
            );
        }
        Ok(first)
    }

    fn mark_flow_failed(&mut self, flow_id: FlowId, reason: &str) -> Result<bool> {
        if let Some(flow) = self.flows.get_mut(&flow_id) {
            let ancestor_pegout_id = flow.ancestor_pegout_id();
            let _span = info_span!(
                "operator_take",
                ancestor_pegout_id = %ancestor_pegout_id,
                operator_take_id = %flow_id,
            )
            .entered();
            flow.mark_failed(reason)?;
            self.retries.cancel(&flow_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Tear down an advance-funds flow declared dead by an admin operator.
    fn fail_flow(&mut self, flow_id: FlowId, reason: &str) -> Result<()> {
        if self.mark_flow_failed(flow_id, reason)? {
            warn!("Admin marked advance-funds flow {flow_id} as failed: {reason}");
        } else {
            warn!("Admin requested fail for unknown advance-funds flow {flow_id}: {reason}");
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn supersede_flows_for_operator_take(
        &mut self,
        trigger_data: &OperatorTakeTriggerData,
        new_flow_id: FlowId,
    ) -> Result<()> {
        let request_key = (trigger_data.committee_id.clone(), trigger_data.slot_id);
        let request_pegout_tx_hash = self.request_pegout_tx_hashes.get(&request_key).cloned();
        let superseded_flows: Vec<_> = self
            .flows
            .values()
            .filter(|flow| {
                !flow.is_terminal()
                    && flow.matches_committee_slot(*trigger_data.committee_id, trigger_data.slot_id)
            })
            .map(|flow| (flow.flow_id(), flow.take_operator_address()))
            .collect();
        let had_superseded_flows = !superseded_flows.is_empty();

        for (old_flow_id, old_operator_address) in superseded_flows {
            warn!(
                old_flow_id = %old_flow_id,
                new_flow_id = %new_flow_id,
                old_operator_address = %old_operator_address,
                new_operator_address = %trigger_data.take_operator_address,
                committee_id = %trigger_data.committee_id,
                slot_id = trigger_data.slot_id,
                "Superseding existing advance funds flow for operator-take slot",
            );
            self.mark_flow_failed(
                old_flow_id,
                &format!("superseded by operator-take flow {new_flow_id}"),
            )?;
        }

        if had_superseded_flows {
            self.cleanup_terminal_flows();
            if let Some(tx_hash) = request_pegout_tx_hash {
                self.request_pegout_tx_hashes.insert(request_key, tx_hash);
            }
        }

        Ok(())
    }

    fn create_flow_for_operator_take_triggered(
        &mut self,
        event: &OperatorTakeTriggeredEvent,
    ) -> Result<()> {
        let committee_id: CommitteeId = event.inner.pegoutInfo.committeeId.into();
        let request_pegout_tx_hash = self
            .request_pegout_tx_hashes
            .get(&(committee_id, event.inner.streamPosition.slotId))
            .cloned();
        let trigger_data = OperatorTakeTriggerData::try_from_event(event, request_pegout_tx_hash)?;
        let committee_id = trigger_data.committee_id.clone();

        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping OperatorTakeTriggered for committee {committee_id} - not a member");
            return Ok(());
        }

        let flow_id = flow_id_from_operator_take_triggered_tx_hash(event.tx_hash);
        if self.flows.contains_key(&flow_id) {
            bail!("Advance funds flow {flow_id} already exists for committee {committee_id}");
        }
        self.supersede_flows_for_operator_take(&trigger_data, flow_id)?;

        let flow = AdvanceFundsFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            self.native_bridge_verifier.clone(),
            self.signaling.clone(),
            Rc::clone(&self.store),
            flow_id,
            trigger_data,
        );
        {
            let ancestor_pegout_id = flow.ancestor_pegout_id();
            let _span = info_span!(
                "operator_take",
                ancestor_pegout_id = %ancestor_pegout_id,
                operator_take_id = %flow_id,
            )
            .entered();

            debug!(
                "Creating advance funds flow {} for committee {} and slot {}",
                flow_id,
                committee_id,
                flow.slot_index()
            );
        }

        self.flows.insert(flow_id, flow);
        self.complete_step(flow_id, StepData::OperatorTakeTriggered)?;

        Ok(())
    }

    fn handle_pegout_registered(&mut self, event: &PegoutRegisteredEvent) -> Result<()> {
        let pegout_registered = event.inner.clone();
        let event_committee_id = pegout_registered.committeeId;
        let event_slot_id = pegout_registered.streamInfo.slotId;

        let Some(flow_id) = self
            .find_flow_by_committee_slot(event_committee_id, event_slot_id)
            .map(|flow| flow.flow_id())
        else {
            trace!(
                "No advance funds flow found for PegoutRegistered with committee_id {event_committee_id} slot_id {event_slot_id}",
            );
            return Ok(());
        };

        self.complete_step(flow_id, StepData::PegoutRegistered(pegout_registered))
    }

    fn complete_step_by_pegout_id(
        &mut self,
        pegout_id: Hash256,
        step_data: StepData,
        event_name: &str,
    ) -> Result<()> {
        let flow_id = {
            let Some(flow) = self.flows.values().find(|flow| flow.matches_pegout(pegout_id)) else {
                trace!("No flow found for {event_name} with pegout_id {pegout_id}");
                return Ok(());
            };
            if let StepData::AdvanceFundsConfirmed(data) = &step_data
                && !flow.matches_operator_take_pubkey(&data.operator_pubkey)
            {
                warn!(
                    event_name = event_name,
                    pegout_id = %pegout_id,
                    flow_id = %flow.flow_id(),
                    received_operator_pubkey = ?data.operator_pubkey,
                    "Ignoring event for superseded operator-take flow",
                );
                return Ok(());
            }
            flow.flow_id()
        };

        info!("{event_name} received for pegout_id {pegout_id}");
        self.complete_step(flow_id, step_data)
    }

    fn handle_reimbursement_kickoff_registered(&mut self, pegout_id: Hash256) -> Result<()> {
        self.complete_step_by_pegout_id(
            pegout_id,
            StepData::ReimbursementKickoffConfirmed,
            "ReimbursementKickoffRegistered",
        )
    }

    fn has_flow_for_pegout_id(&self, pegout_id: Hash256) -> bool {
        self.flows.values().any(|flow| flow.matches_pegout(pegout_id))
    }

    /// Find the flow whose trigger matches `(committee_id, slot_id)`, if any.
    /// Callers decide what to do with the flow (mutate it, check existence,
    /// etc).
    fn find_flow_by_committee_slot(
        &mut self,
        committee_id: u128,
        slot_id: u64,
    ) -> Option<&mut AdvanceFundsFlow<CG, BC, S>> {
        self.flows.values_mut().find(|flow| flow.matches_committee_slot(committee_id, slot_id))
    }

    fn cleanup_terminal_flow_state(&mut self) {
        let terminal_flows: Vec<&AdvanceFundsFlow<CG, BC, S>> =
            self.flows.values().filter(|flow| flow.is_terminal()).collect();

        for flow in terminal_flows {
            let flow_id = flow.flow_id();
            self.retries.cancel(&flow_id);
            self.request_pegout_tx_hashes
                .retain(|(cid, sid), _| !flow.matches_committee_slot(**cid, *sid));
        }
    }

    fn cleanup_terminal_flows(&mut self) {
        self.cleanup_terminal_flow_state();

        let terminal_flow_ids: Vec<_> = self
            .flows
            .values()
            .filter(|flow| flow.is_terminal())
            .map(AdvanceFundsFlow::flow_id)
            .collect();

        for flow_id in terminal_flow_ids {
            debug!("Removing terminal advance funds flow {flow_id}");
            if let Err(err) = self.store.delete_flow(&StoreKey::OperatorTakeFlow(flow_id.value())) {
                error!("Failed to remove operator-take flow {flow_id} from persistence: {err}");
            }
        }
        self.flows.retain(|_, flow| !flow.is_terminal());
    }

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::PegoutRequested(pegout_requested) => {
                let committee_id: CommitteeId = pegout_requested.inner.committeeId.try_into()?;
                self.request_pegout_tx_hashes.insert(
                    (committee_id, pegout_requested.inner.slotId),
                    pegout_requested.tx_hash.to_string(),
                );
            }
            RskPegManagerEvents::OperatorTakeTriggered(op_take) => {
                info!(
                    "Processing confirmed OperatorTakeTriggered event: flow tx {:?}",
                    op_take.tx_hash
                );
                self.create_flow_for_operator_take_triggered(op_take)?;
            }
            RskPegManagerEvents::AdvanceFundsRegistered(e) => {
                let ev = &e.inner;
                let data = advance_funds_registered_from_event(ev)?;
                self.complete_step_by_pegout_id(
                    Hash256::from(ev.pegoutId),
                    StepData::AdvanceFundsConfirmed(data),
                    "AdvanceFundsRegistered",
                )?;
            }
            RskPegManagerEvents::ReimbursementKickoffRegistered(e) => {
                self.handle_reimbursement_kickoff_registered(Hash256::from(e.inner.pegoutId))?;
            }
            RskPegManagerEvents::PegoutRegistered(pegout_registered) => {
                self.handle_pegout_registered(pegout_registered)?;
            }
            _ => {
                trace!("AdvanceFundsFlowProcessor ignoring confirmed event {event:?}");
            }
        }

        self.cleanup_terminal_flows();
        Ok(())
    }

    fn build_pegout_requested_event_info(
        event: &crate::types::PegoutRequestedEvent,
    ) -> (String, EventStatus, common_core::types::BlockNumber, RskPegManagerEvents) {
        (
            format!("advance-funds-pegout-requested-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::PegoutRequested(event.clone()),
        )
    }

    fn build_operator_take_triggered_event_info(
        event: &OperatorTakeTriggeredEvent,
    ) -> (String, EventStatus, common_core::types::BlockNumber, RskPegManagerEvents) {
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
                debug!("Advance funds RSK event confirmed, removing pending {key}");
                trace!("Advance funds event data: {:?}", event.get_data());
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        self.cleanup_terminal_flows();
        Ok(())
    }

    fn complete_step(&mut self, flow_id: FlowId, step_data: StepData) -> Result<()> {
        let Some(ancestor_pegout_id) =
            self.flows.get(&flow_id).map(AdvanceFundsFlow::ancestor_pegout_id)
        else {
            trace!("Ignoring step delivery for flow {flow_id} - no matching flow");
            return Ok(());
        };
        let _span = info_span!(
            "operator_take",
            ancestor_pegout_id = %ancestor_pegout_id,
            operator_take_id = %flow_id,
        )
        .entered();

        let Some(flow) = self.flows.get_mut(&flow_id) else {
            trace!("Ignoring step delivery for flow {flow_id} - no matching flow");
            return Ok(());
        };

        match flow.complete_step(step_data)? {
            StepOutcome::Done | StepOutcome::NoOp => {}
            StepOutcome::Retry { reason } => {
                let attempt = self.retries.current_attempt(&flow_id).saturating_add(1);
                self.schedule_retry(flow_id, attempt, &reason);
            }
        }

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

        let Some(flow_id) = self
            .flows
            .values()
            .find(|flow| flow.matches_pegout(pegout_id))
            .map(AdvanceFundsFlow::flow_id)
        else {
            trace!("Ignoring funds_advance_spv for pegout_id {pegout_id} - no matching flow");
            return Ok(());
        };

        self.complete_step(flow_id, StepData::AdvanceFundsSPV(spv_data.clone()))
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

        let Some(flow_id) = self
            .flow_by_committee_slot(notification.committee_id, notification.slot_index)?
            .map(|flow| flow.flow_id())
        else {
            trace!(
                "Ignoring ReimbursementKickoff SPV for committee {} slot {} - no matching flow",
                notification.committee_id, notification.slot_index
            );
            return Ok(());
        };

        let spv_proof = notification.spv_proof.clone().ok_or_else(|| {
            anyhow!("ReimbursementKickoff SPV notification missing spv_proof data")
        })?;

        self.complete_step(flow_id, StepData::ReimbursementKickoffSPV(spv_proof))
    }

    fn handle_operator_take_spv_notification(
        &mut self,
        notification: &UnionSPVNotification,
    ) -> Result<()> {
        info!(
            "Received OperatorTake SPV notification - committee_id: {}, slot_index: {}, txid: {}",
            notification.committee_id, notification.slot_index, notification.txid
        );

        let Some(flow_id) = self
            .flow_by_committee_slot(notification.committee_id, notification.slot_index)?
            .map(|flow| flow.flow_id())
        else {
            trace!(
                "Ignoring OperatorTake SPV for committee {} slot {} - no matching flow",
                notification.committee_id, notification.slot_index
            );
            return Ok(());
        };

        let spv_proof = notification
            .spv_proof
            .clone()
            .ok_or_else(|| anyhow!("OperatorTake SPV notification missing spv_proof data"))?;

        self.complete_step(flow_id, StepData::OperatorTakeSPV(spv_proof))
    }
}

impl<CG, BC, S> EventProcessor for AdvanceFundsFlowProcessor<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: common_broker::broker::BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        self.cleanup_terminal_flows();

        // Advance-funds flows have no other user-driven entry points; we only act
        // on the admin "fail flow" lever.
        if let UserRequests::Admin(AdminRequest::FailFlow { kind, flow_id, reason }) = req
            && *kind == FlowKind::AdvanceFunds
        {
            self.fail_flow(*flow_id, reason)?;
        }
        self.persist_processor_state()
    }

    #[allow(clippy::too_many_lines)]
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        self.cleanup_terminal_flows();

        match event {
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                debug!(
                    "Advance funds flow processor received SetupCompleted for program_id: {program_id}",
                );
                // Route by `bitvmx_protocol_id`: BitVMX's `program_id` matches
                // the cached protocol id, not the coordinator's `FlowId`. Same
                // uniqueness invariant as `flow_by_committee_slot`: at most one
                // flow per protocol id.
                let target = (*program_id).into();
                let mut matches =
                    self.flows.values().filter(|flow| flow.bitvmx_protocol_id() == target);
                let first = matches.next().map(AdvanceFundsFlow::flow_id);
                if matches.next().is_some() {
                    bail!(
                        "Invariant violation: multiple advance-funds flows hold BitVMX protocol id \
                         {program_id} on SetupCompleted; refusing to route",
                    );
                }
                if let Some(flow_id) = first {
                    self.complete_step(flow_id, StepData::SetupCompleted)?;
                } else {
                    trace!(
                        "Ignoring SetupCompleted for program {program_id}: no matching advance funds flow",
                    );
                }
            }
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("Received CommInfo from BitVMX req_id: {req_id}, comm_info: {comm_info:?}");
                // CommInfo isn't program-id-scoped, so we route by step: only
                // flows currently at `GetBitVmxCommInfo` receive it. Limitation
                // comes from BitVMX message types not carrying a flow/program
                // id back on this reply.
                let waiting_flow_ids: Vec<FlowId> = self
                    .flows
                    .iter()
                    .filter(|(_, flow)| flow.is_waiting_comm_info())
                    .map(|(flow_id, _)| *flow_id)
                    .collect();
                for flow_id in waiting_flow_ids {
                    debug!("Advance funds flow {flow_id} received comm info");
                    self.complete_step(flow_id, StepData::CommInfo(comm_info.clone()))?;
                }
            }
            OutgoingBitVMXApiMessages::TransactionInfo(program_id, tx_name, transaction) => {
                let txid = transaction.compute_txid();
                debug!(
                    "Received BitVMX TransactionInfo: program_id={program_id}, tx_name={tx_name}, txid={txid}",
                );
                let operator_take_prefix = format!("{OPERATOR_TAKE_TX}_");
                if tx_name.starts_with(&operator_take_prefix) {
                    let flow_id = self
                        .flows
                        .values()
                        .find(|flow| flow.matches_accept_pegin_pid(program_id))
                        .map(AdvanceFundsFlow::flow_id);
                    if let Some(flow_id) = flow_id {
                        debug!(
                            "Routing TransactionInfo to advance funds flow {flow_id}: {tx_name}",
                        );
                        self.complete_step(flow_id, StepData::OperatorTakeTransactionInfo(txid))?;
                    } else {
                        trace!(
                            "Ignoring TransactionInfo for {tx_name}: no advance funds flow for program_id={program_id}",
                        );
                    }
                } else {
                    trace!("Ignoring BitVMX TransactionInfo for unrelated tx: {tx_name}");
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
                    trace!("AdvanceFundsFlowProcessor ignoring Variable with name: {var_name}");
                }
            }
            _ => {
                trace!("AdvanceFundsFlowProcessor ignoring BitVMX event {event:?}");
            }
        }
        self.persist_processor_state()
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        self.cleanup_terminal_flows();

        if self.required_confirmations == 0 {
            self.process_confirmed_rsk_event(event)?;
            return self.persist_processor_state();
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PegoutRequested(e) => Self::build_pegout_requested_event_info(e),
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
                if self.find_flow_by_committee_slot(event_committee_id, event_slot_id).is_none() {
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
                warn!("Tried to remove non-existing pending advance funds event with id {id}");
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

        self.persist_processor_state()
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.cleanup_terminal_flows();

        self.process_block_confirmations(block)?;
        self.handle_retry_tick();
        self.persist_processor_state()
    }

    fn shutdown(&mut self) {
        info!("Shutting down AdvanceFundsFlowProcessor");
        self.flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.retries.clear();
    }

    fn active_flows(&self) -> Vec<crate::event_processor::FlowDetails> {
        self.flows
            .values()
            .filter(|f| !f.is_terminal())
            .map(AdvanceFundsFlow::get_flow_details)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::str::FromStr;

    use alloy_primitives::{Bytes, FixedBytes, I256, U256, U256 as AlloyU256};
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{PublicKey, Transaction};
    use common_bitvmx::bitvmx_types::{
        AdvanceFundsRegistered, BtcTxSPVProof, IncomingBitVMXApiMessages,
        OutgoingBitVMXApiMessages, ParticipantRole, UnionSPVNotification, UnionTxType,
    };
    use common_broker::broker::MockBrokerClientApi;
    use common_core::types::{Address, BlockHash, BlockNumber, CommitteeId, Hash256, TxHash};
    use primitive_types::{H160, H256};
    use union_contracts::bindings::pegout_manager::PegoutManager::{
        BitcoinSignatureData, BtcTransaction, OperatorTakeTriggered, PegoutRequested,
        PegoutTempInfo, StreamPosition,
    };
    use uuid::Uuid;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::store::{CoordinatorStore, CoordinatorStoreApi, TestStorePath};

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;

    /// Build a contracts mock whose `my_address()` matches (selected operator)
    /// or differs from (non-selected operator) the trigger's `take_operator_address`.
    fn test_contracts(is_selected: bool) -> MockRskContractsGatewayApi {
        let mut contracts = MockRskContractsGatewayApi::new();
        let addr = if is_selected { 33 } else { 44 };
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(addr)));
        contracts
    }

    fn test_trigger_data(committee_id: Uuid, slot_index: usize) -> OperatorTakeTriggerData {
        OperatorTakeTriggerData {
            pegout_txid: Hash256::from(H256::from_low_u64_be(11)),
            pegout_id: Hash256::from(H256::from_low_u64_be(22)),
            committee_id: CommitteeId::from(committee_id.as_u128()),
            slot_id: slot_index as u64,
            slot_index,
            request_pegout_tx_hash: None,
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

    fn test_global_context(committee_id: Uuid) -> GlobalContext {
        let global_context = GlobalContext::new();
        global_context
            .my_committees()
            .add(CommitteeId::from(committee_id.as_u128()), ParticipantRole::Prover);
        global_context
    }

    fn test_pubkey(seed: u8) -> PublicKey {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let secret_key =
            bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).expect("valid test secret key");
        PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key))
    }

    fn test_xonly_pubkey(seed: u8) -> FixedBytes<32> {
        let pubkey = test_pubkey(seed);
        FixedBytes::from_slice(&pubkey.inner.serialize()[1..])
    }

    fn test_alloy_address(value: u64) -> alloy_primitives::Address {
        let mut bytes = [0u8; 20];
        bytes[12..].copy_from_slice(&value.to_be_bytes());
        alloy_primitives::Address::from(bytes)
    }

    fn test_operator_take_triggered_event(
        committee_id: Uuid,
        slot_index: usize,
        take_operator_address: u64,
        operator_key_seed: u8,
        tx_hash_value: u64,
    ) -> OperatorTakeTriggeredEvent {
        OperatorTakeTriggeredEvent {
            inner: OperatorTakeTriggered {
                pegoutTxid: FixedBytes::from_slice(H256::from_low_u64_be(11).as_bytes()),
                pegoutInfo: PegoutTempInfo {
                    userPubKey: Bytes::from(test_pubkey(9).inner.serialize().to_vec()),
                    createdAt: U256::ZERO,
                    operatorTakeUpdatedAt: U256::ZERO,
                    committeeId: committee_id.as_u128(),
                    takeOperatorAddress: test_alloy_address(take_operator_address),
                    operatorTakePubKey: test_xonly_pubkey(operator_key_seed),
                    operatorDisputePubKey: FixedBytes::ZERO,
                    pegoutId: FixedBytes::from_slice(H256::from_low_u64_be(22).as_bytes()),
                    advanceFundsBlockNumber: I256::ZERO,
                    reimbursementKickoffTxid: FixedBytes::ZERO,
                },
                streamPosition: StreamPosition {
                    streamId: 0,
                    packetNumber: 0,
                    slotId: slot_index as u64,
                    pegStatus: 0,
                },
                updatedAt: U256::ZERO,
                expireAt: U256::from(1u64),
            },
            block_number: 1.into(),
            block_hash: Hash256::from(H256::from_low_u64_be(100 + tx_hash_value)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(tx_hash_value)),
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

    fn store_backed_processor(
        store: &Rc<CoordinatorStore>,
    ) -> AdvanceFundsFlowProcessor<MockRskContractsGatewayApi, MockBitVmxBroker, CoordinatorStore>
    {
        AdvanceFundsFlowProcessor::new(
            Rc::new(MockRskContractsGatewayApi::new()),
            RuntimeSync::new().expect("failed to create runtime sync"),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
            Rc::new(Signaling::new("/tmp", "disabled")),
            store,
            0,
            1,
            NativeBridgeVerifier::Dummy,
        )
    }

    fn test_pegout_requested_event(
        committee_id: u128,
        slot_id: u64,
    ) -> crate::types::PegoutRequestedEvent {
        crate::types::EventWithBlock {
            inner: PegoutRequested {
                userPubKey: Bytes::from(vec![0x03; 33]),
                committeeId: AlloyU256::from(committee_id),
                pegoutSignatureData: BitcoinSignatureData {
                    tx: BtcTransaction { version: 2, inputs: vec![], outputs: vec![], locktime: 0 },
                    txid: FixedBytes::from([1u8; 32]),
                    signatureHash: FixedBytes::from([2u8; 32]),
                    signatureMessage: Bytes::from(vec![3u8; 32]),
                },
                streamId: 0,
                packetNumber: 0,
                slotId: slot_id,
                amount: 100_000,
            },
            block_number: BlockNumber::from(1),
            block_hash: BlockHash::from(H256::from_low_u64_be(2)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(3)),
        }
    }

    fn saved_flow_state(
        committee_id: Uuid,
        slot_index: usize,
        flow_id: FlowId,
        step: Steps,
        pending_retry: bool,
    ) -> FlowContext {
        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            test_trigger_data(committee_id, slot_index),
            step,
        );
        if pending_retry { flow.into_saved_state_pending_retry() } else { flow.into_saved_state() }
    }

    #[test]
    fn request_pegout_tx_hashes_round_trip_through_store() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let mut processor = store_backed_processor(&store);
        let committee_uuid = Uuid::new_v4();
        let committee_id = CommitteeId::from(committee_uuid.as_u128());
        let slot_id = 4;
        let event = test_pegout_requested_event(committee_uuid.as_u128(), slot_id);
        let tx_hash = event.tx_hash.to_string();

        processor
            .process_new_rsk_event(&RskPegManagerEvents::PegoutRequested(event))
            .expect("PegoutRequested should persist advance-funds processor state");
        drop(processor);

        let restored = store_backed_processor(&store);
        assert_eq!(restored.request_pegout_tx_hashes.get(&(committee_id, slot_id)), Some(&tx_hash));
    }

    #[test]
    fn operator_take_flow_round_trip_through_store() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let state =
            saved_flow_state(committee_id, 4, flow_id, Steps::RegisterOrWaitRskAdvanceFunds, false);

        store
            .save_flow(&StoreKey::OperatorTakeFlow(flow_id.value()), state)
            .expect("operator-take flow state should persist");

        let restored = store_backed_processor(&store);
        assert_eq!(
            restored.flows.get(&flow_id).expect("flow should restore").current_step(),
            Steps::RegisterOrWaitRskAdvanceFunds
        );
    }

    #[test]
    fn pending_retry_flow_rearms_retry_when_not_scheduled() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let state =
            saved_flow_state(committee_id, 4, flow_id, Steps::RegisterOrWaitRskAdvanceFunds, true);

        store
            .save_flow(&StoreKey::OperatorTakeFlow(flow_id.value()), state)
            .expect("operator-take flow state should persist");

        let restored = store_backed_processor(&store);
        assert!(restored.retries.is_scheduled(&flow_id));
        assert_eq!(restored.retries.current_attempt(&flow_id), 1);
    }

    #[test]
    fn pending_retry_does_not_reset_restored_retry_attempt() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let flow_state =
            saved_flow_state(committee_id, 4, flow_id, Steps::RegisterOrWaitRskAdvanceFunds, true);
        let mut processor_state = AdvanceFundsProcessorState {
            events_confirming: HashMap::new(),
            retries: RetryTracker::new(),
            request_pegout_tx_hashes: Vec::new(),
        };
        processor_state.retries.schedule(flow_id, 3, 7);

        store
            .save_flow(&StoreKey::OperatorTakeFlow(flow_id.value()), flow_state)
            .expect("operator-take flow state should persist");
        store
            .save_flow(&StoreKey::AdvanceFundsProcessorState, processor_state)
            .expect("processor state should persist");

        let restored = store_backed_processor(&store);
        assert!(restored.retries.is_scheduled(&flow_id));
        assert_eq!(restored.retries.current_attempt(&flow_id), 3);
    }

    #[test]
    fn cleanup_terminal_flows_deletes_operator_take_flow_state() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let state = saved_flow_state(committee_id, 4, flow_id, Steps::Failed, false);

        store
            .save_flow(&StoreKey::OperatorTakeFlow(flow_id.value()), state)
            .expect("operator-take flow state should persist");
        let mut processor = store_backed_processor(&store);
        assert!(processor.flows.contains_key(&flow_id));

        processor.cleanup_terminal_flows();

        assert!(!processor.flows.contains_key(&flow_id));
        let stored: Option<FlowContext> = store
            .load_flow(&StoreKey::OperatorTakeFlow(flow_id.value()))
            .expect("operator-take flow lookup should succeed");
        assert!(stored.is_none());
    }

    #[test]
    fn processor_state_snapshot_restores_advance_funds_runtime_context() {
        let committee_id = Uuid::new_v4();
        let slot_id = 4_u64;
        let flow_id = FlowId::from_random();

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.retries.schedule(flow_id, 3, 7);
        processor
            .request_pegout_tx_hashes
            .insert((CommitteeId::from(committee_id.as_u128()), slot_id), "0xabc".into());

        let mut confirmable_event = ConfirmableEventWithData::new(
            "operator-take-triggered-test".to_string(),
            5,
            processor.blockchain_view.clone(),
            RskPegManagerEvents::UnknownEvent,
        );
        confirmable_event.start_confirming(BlockNumber::from(1)).unwrap();
        processor.events_confirming.insert(confirmable_event.id(), confirmable_event);

        let snapshot = processor.snapshot_processor_state();
        let mut restored = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        restored.apply_processor_state(snapshot).unwrap();

        assert_eq!(restored.events_confirming.len(), 1);
        assert!(restored.flows.is_empty());
        assert!(restored.retries.is_scheduled(&flow_id));
        assert_eq!(restored.retries.current_attempt(&flow_id), 3);
        assert_eq!(
            restored
                .request_pegout_tx_hashes
                .get(&(CommitteeId::from(committee_id.as_u128()), slot_id)),
            Some(&"0xabc".to_string())
        );
    }

    #[test]
    fn advance_funds_confirmation_notifies_passive_followers() {
        let committee_id = Uuid::new_v4();
        let slot_index = 4;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let mut flow_broker = MockBitVmxBroker::new();
        flow_broker.expect_send().times(1).returning(|_| Ok(true));

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(flow_broker),
            flow_id,
            trigger_data.clone(),
            Steps::RegisterOrWaitRskAdvanceFunds,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        let registered_data = AdvanceFundsRegistered {
            committee_id,
            slot_index,
            txid: proof_txid(),
            pegout_id: trigger_data.pegout_id.value().as_bytes().to_vec(),
            operator_pubkey: trigger_data.operator_take_pubkey,
        };

        processor
            .complete_step_by_pegout_id(
                trigger_data.pegout_id,
                StepData::AdvanceFundsConfirmed(registered_data),
                "AdvanceFundsRegistered",
            )
            .expect("passive follower should notify BitVMX after advance funds confirmation");

        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::SetVarBitVmxAdvanceFundsRegistered);
        assert!(flow.has_advance_funds_registered());
    }

    #[test]
    fn reimbursement_kickoff_registered_at_terminal_wait_surfaces_error() {
        // A late or duplicate `ReimbursementKickoffRegistered` arriving at a
        // non-selected flow already at the terminal `WaitRskPegoutRegistered`
        // is treated as unexpected and surfaces as Err (e.g. log-indexer
        // re-emission on restart — should be investigated).
        let committee_id = Uuid::new_v4();
        let slot_index = 5;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data.clone(),
            Steps::RegisterOrWaitRskOperatorTake,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        let result = processor.handle_reimbursement_kickoff_registered(trigger_data.pegout_id);

        assert!(result.is_err(), "late ReimbursementKickoffRegistered should surface as error");
        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskOperatorTake);
    }

    #[test]
    fn reimbursement_kickoff_spv_at_terminal_wait_drops_silently() {
        // BitVMX re-emits SPVs on every block-confirmation update (notify_news).
        // A `ReimbursementKickoffSPV` arriving at a flow already past the SPV's
        // handler step is stale and must drop silently — no broker send, no
        // error, no state change. The previous behaviour (surfacing as Err)
        // produced a firehose of bails that crashed the broker under
        // rate-limiting.
        let committee_id = Uuid::new_v4();
        let slot_index = 8;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskOperatorTake,
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
            spv_proof: Some(proof),
            tx_type: UnionTxType::ReimbursementKickoff,
        };

        processor
            .handle_union_spv_notification(&notification)
            .expect("stale ReimbursementKickoffSPV should drop silently");
        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskOperatorTake);
    }

    #[test]
    fn operator_take_spv_at_terminal_wait_drops_silently() {
        // Same at-least-once delivery semantics as ReimbursementKickoffSPV:
        // stale `OperatorTakeSPV` at a post-handler step is a no-op, not an
        // error.
        let committee_id = Uuid::new_v4();
        let slot_index = 9;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskOperatorTake,
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
            spv_proof: Some(proof),
            tx_type: UnionTxType::OperatorTake,
        };

        processor
            .handle_union_spv_notification(&notification)
            .expect("stale OperatorTakeSPV should drop silently");
        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskOperatorTake);
    }

    #[test]
    fn reimbursement_kickoff_registered_advances_selected_operator_path() {
        let committee_id = Uuid::new_v4();
        let slot_index = 6;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(true)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data.clone(),
            Steps::RegisterOrWaitRskReimbursementKickoff,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        processor
            .handle_reimbursement_kickoff_registered(trigger_data.pegout_id)
            .expect("selected operator should advance after reimbursement kickoff confirmation");

        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::WaitBitVmxOperatorTakeSpv);
    }

    #[test]
    fn reimbursement_kickoff_registered_at_early_step_surfaces_error_for_selected() {
        // A selected operator at an early step receiving `ReimbursementKickoffRegistered`
        // is a state-divergence anomaly (only the selected operator drives this event,
        // and only after their own kickoff registration submission). Crash to surface.
        let committee_id = Uuid::new_v4();
        let slot_index = 7;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(true)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data.clone(),
            Steps::RegisterOrWaitRskAdvanceFunds,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        let result = processor.handle_reimbursement_kickoff_registered(trigger_data.pegout_id);

        assert!(result.is_err(), "early ReimbursementKickoffRegistered should surface as error");
        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskAdvanceFunds);
    }

    fn proof_txid() -> bitcoin::Txid {
        test_spv_proof().tx.compute_txid()
    }

    #[test]
    fn second_operator_take_trigger_supersedes_existing_slot_flow() {
        let committee_id = Uuid::new_v4();
        let slot_index = 4;
        let first_event = test_operator_take_triggered_event(committee_id, slot_index, 33, 1, 101);
        let second_event = test_operator_take_triggered_event(committee_id, slot_index, 44, 2, 202);
        let first_flow_id = flow_id_from_operator_take_triggered_tx_hash(first_event.tx_hash);
        let second_flow_id = flow_id_from_operator_take_triggered_tx_hash(second_event.tx_hash);

        let mut broker = MockBitVmxBroker::new();
        broker.expect_send().times(1).returning(|_| Ok(true));

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(test_contracts(true)),
            Rc::new(broker),
            test_global_context(committee_id),
            test_store(),
        );

        processor
            .create_flow_for_operator_take_triggered(&first_event)
            .expect("first trigger should create a flow");
        processor.retries.schedule(first_flow_id, 1, 1);

        processor
            .create_flow_for_operator_take_triggered(&second_event)
            .expect("second trigger should supersede the first flow");

        assert!(!processor.flows.contains_key(&first_flow_id), "old flow removed");
        assert!(!processor.retries.is_scheduled(&first_flow_id), "old retry removed");
        assert_eq!(processor.flows.len(), 1, "only the replacement flow remains");

        let flow = processor.flows.get(&second_flow_id).expect("new flow should exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskAdvanceFunds);
    }

    #[test]
    fn advance_funds_registered_for_superseded_operator_is_ignored() {
        let committee_id = Uuid::new_v4();
        let slot_index = 4;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let mut flow_broker = MockBitVmxBroker::new();
        flow_broker.expect_send().returning(|_| Ok(true));

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(false)),
            Rc::new(flow_broker),
            flow_id,
            trigger_data.clone(),
            Steps::RegisterOrWaitRskAdvanceFunds,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
            test_store(),
        );
        processor.flows.insert(flow_id, flow);

        let registered_data = AdvanceFundsRegistered {
            committee_id,
            slot_index,
            txid: proof_txid(),
            pegout_id: trigger_data.pegout_id.value().as_bytes().to_vec(),
            operator_pubkey: test_pubkey(2),
        };

        processor
            .complete_step_by_pegout_id(
                trigger_data.pegout_id,
                StepData::AdvanceFundsConfirmed(registered_data),
                "AdvanceFundsRegistered",
            )
            .expect("stale confirmation should be ignored");

        let flow = processor.flows.get(&flow_id).expect("flow should still exist");
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskAdvanceFunds);
        assert!(!flow.has_advance_funds_registered());
    }

    #[test]
    fn cleanup_terminal_flows_removes_advance_funds_flow_and_retry_state() {
        let committee_id = Uuid::new_v4();
        let slot_index = 4;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::Failed,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);
        processor.retries.schedule(flow_id, 1, 1);
        processor.request_pegout_tx_hashes.insert(
            (CommitteeId::from(committee_id.as_u128()), slot_index as u64),
            "0xrequest".to_string(),
        );

        processor.cleanup_terminal_flows();

        assert!(!processor.flows.contains_key(&flow_id));
        assert!(!processor.retries.is_scheduled(&flow_id));
        assert!(
            !processor
                .request_pegout_tx_hashes
                .contains_key(&(CommitteeId::from(committee_id.as_u128()), slot_index as u64))
        );
    }

    #[test]
    fn retry_tick_after_success_event_does_not_reschedule() {
        // If a success event (e.g. AdvanceFundsConfirmed) arrives between a
        // retry's schedule and its tick, the flow has advanced past the
        // register step. When the tick fires, `complete_step` errors with
        // "invalid state transition" (not "missing confirmations"), so the
        // retry handler logs and stops — the retry is consumed and not
        // rescheduled.
        let committee_id = Uuid::new_v4();
        let slot_index = 4;
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, slot_index);

        // Flow already past the register step (the success event arrived first).
        let flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(true)),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::SetVarBitVmxAdvanceFundsRegistered,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(flow_id, flow);

        // Simulate a stale retry scheduled before the success event landed.
        processor.retries.schedule(flow_id, 1, 1);
        assert!(processor.retries.is_scheduled(&flow_id));

        processor.handle_retry_tick();

        assert!(
            !processor.retries.is_scheduled(&flow_id),
            "stale retry should be consumed by the tick and not rescheduled"
        );
    }

    #[test]
    fn cleanup_does_not_disturb_other_slot_flows_for_same_committee() {
        // Two flows for the same committee on different slots. Cleaning up
        // the terminal one must not drop the other's retry state or its entry
        // in request_pegout_tx_hashes.
        let committee_id = Uuid::new_v4();
        let slot_terminal = 4;
        let slot_active = 5;

        let terminal_flow_id = FlowId::from_random();
        let active_flow_id = FlowId::from_random();

        let terminal_flow = AdvanceFundsFlow::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            terminal_flow_id,
            test_trigger_data(committee_id, slot_terminal),
            Steps::Failed,
        );
        let active_flow = AdvanceFundsFlow::new_for_test(
            Rc::new(test_contracts(true)),
            Rc::new(MockBitVmxBroker::new()),
            active_flow_id,
            test_trigger_data(committee_id, slot_active),
            Steps::RegisterOrWaitRskAdvanceFunds,
        );

        let mut processor = AdvanceFundsFlowProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
        );
        processor.flows.insert(terminal_flow_id, terminal_flow);
        processor.flows.insert(active_flow_id, active_flow);

        // Retry state for both flows.
        processor.retries.schedule(terminal_flow_id, 1, 5);
        processor.retries.schedule(active_flow_id, 1, 5);

        // request_pegout_tx_hashes entries for both slots.
        let cid = CommitteeId::from(committee_id.as_u128());
        processor
            .request_pegout_tx_hashes
            .insert((cid.clone(), slot_terminal as u64), "0xterminal".to_string());
        processor
            .request_pegout_tx_hashes
            .insert((cid.clone(), slot_active as u64), "0xactive".to_string());

        processor.cleanup_terminal_flows();

        assert!(!processor.flows.contains_key(&terminal_flow_id), "terminal flow removed");
        assert!(processor.flows.contains_key(&active_flow_id), "active flow preserved");

        assert!(
            !processor.retries.is_scheduled(&terminal_flow_id),
            "terminal flow's retry removed"
        );
        assert!(processor.retries.is_scheduled(&active_flow_id), "active flow's retry preserved");

        assert!(
            !processor.request_pegout_tx_hashes.contains_key(&(cid.clone(), slot_terminal as u64)),
            "terminal slot's tx hash removed"
        );
        assert!(
            processor.request_pegout_tx_hashes.contains_key(&(cid, slot_active as u64)),
            "active slot's tx hash preserved"
        );
    }
}
