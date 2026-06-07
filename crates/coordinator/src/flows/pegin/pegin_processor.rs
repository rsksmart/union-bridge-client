use std::any::type_name_of_val;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use alloy_primitives::FixedBytes;
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    BitVmxProtocolId, BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
    PeginAcceptedMessage, RSK_PEGIN_TAG, TransactionStatus, VariableTypes,
    accept_pegin_protocol_id,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{BlockNumber, CommitteeId, Hash256, RskBlockAndUncles, TxIdParser};
use serde::{Deserialize, Serialize};
use tracing::span::Span;
use tracing::{debug, error, info, info_span, instrument, trace, warn};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use union_contracts::bindings::pegin_manager::PeginManager::PeginRequested;
use uuid::Uuid;

use crate::blockchain_tracker::{
    BlockchainView, ConfirmableEventWithData, ConfirmableEventWithDataSnapshot,
};
use crate::event_processor::EventProcessor;
use crate::flows::btc_signature::btc_signature_lifecycle::BtcSignatureLifeCycle;
use crate::flows::btc_signature::btc_signature_subflow::{
    BaseBtcSignatureSubFlow, BtcSignatureSubFlowApi, BtcSignatureSubFlowFactory,
    BtcSignatureSubFlowFactoryApi, BtcSignatureSubFlowSnapshot, ParentSpan,
};
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::common::{FlowId, GlobalContext, Signaling};
use crate::flows::pegin::pegin_flow::{
    PeginFlow, State, StepData, Steps, flow_id_from_request_pegin_txid,
};
use crate::store::{CoordinatorStoreApi, StoreKey, StorePrefix, restore_flows};
use crate::types::{
    AdminRequest, AllOperatorTakeTxidsAddedEvent, EventStatus, FlowKind, PeginAcceptedEvent,
    PeginRequestedEvent, RegisterSignaturesBitVmxData, RskPegManagerEvents, TickScheduler,
    UserRequests,
};

const PEGIN_ACCEPTED_INPUT_MSG: &str = "pegin_accepted";
const OPERATOR_TAKE_TX_PREFIX: &str = "OPERATOR_TAKE_TX";
const OPERATOR_WON_TX_PREFIX: &str = "OPERATOR_WON_TX";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PeginProcessorState {
    events_confirming: HashMap<String, ConfirmableEventWithDataSnapshot>,
    tx_status_scheduler: TickScheduler<FlowId>,
    pegin_request_tracker: HashSet<Txid>,
    pending_pegin_requested: HashMap<Txid, PeginRequestedEvent>,
    pending_all_operator_take_txids_added: HashMap<Txid, AllOperatorTakeTxidsAddedEvent>,
    pending_pegin_accepted: HashMap<Txid, PeginAcceptedEvent>,
    unconfirmed_pegin_requests: HashMap<String, (BtcTxSPVProof, i16)>,
    pegin_retry_scheduler: TickScheduler<String>,
    unconfirmed_accept_pegin: HashMap<FlowId, i16>,
    accept_pegin_retry_scheduler: TickScheduler<FlowId>,
    signature_flows: HashMap<Uuid, BtcSignatureSubFlowSnapshot>,
}

fn is_missing_native_bridge_confirmations(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(domain_err) = cause.downcast_ref::<DomainErrors>() {
            matches!(domain_err, DomainErrors::MissingConfirmationsOnNativeBridge(_))
        } else {
            false
        }
    })
}

fn record_pegin_id(flow_id: &FlowId) {
    Span::current().record("pegin_id", tracing::field::display(flow_id));
}

fn is_pegin_already_accepted(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(domain_err) = cause.downcast_ref::<DomainErrors>() {
            matches!(domain_err, DomainErrors::PeginAlreadyAccepted(_))
        } else {
            false
        }
    })
}

/// Processor that manages multiple pegin flow state machines
pub(crate) struct PeginFlowProcessor<CG, BC, BSF, FactoryBSF, S>
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
    /// Pegin flows are keyed by their canonical id (`FlowId`).
    pegin_flows: HashMap<FlowId, PeginFlow<CG, BC, S>>,
    /// BTC signature subflows are keyed by the `BitVMX` protocol id — that's
    /// what the subflow API consumes and what `BitVMX` events for the
    /// signature program carry.
    signature_flows: HashMap<Uuid, BSF>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    tx_status_scheduler: TickScheduler<FlowId>,
    pegin_request_tracker: HashSet<Txid>,
    pending_pegin_requested: HashMap<Txid, PeginRequestedEvent>,
    pending_all_operator_take_txids_added: HashMap<Txid, AllOperatorTakeTxidsAddedEvent>,
    pending_pegin_accepted: HashMap<Txid, PeginAcceptedEvent>,
    // For retry logic when native bridge lacks confirmations
    unconfirmed_pegin_requests: HashMap<String, (BtcTxSPVProof, i16)>,
    pegin_retry_scheduler: TickScheduler<String>,
    unconfirmed_accept_pegin: HashMap<FlowId, i16>,
    accept_pegin_retry_scheduler: TickScheduler<FlowId>,
    store: Rc<S>,
    signaling: Rc<Signaling>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    required_confirmations: u32,
    btc_confirmations: u32,
    btc_status_retry_blocks: u32,
}

impl<CG, BC, S>
    PeginFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        store: &Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        required_confirmations: u32,
        btc_confirmations: u32,
        btc_status_retry_blocks: u32,
    ) -> Self {
        let factory = BtcSignatureSubFlowFactory::new(
            Rc::clone(&contracts_gateway),
            rt_sync.clone(),
            required_confirmations,
        );

        // Subscribe to BitVMX pegin events
        Self::subscribe_to_bitvmx_pegin_events(bitvmx_broker.as_ref(), btc_confirmations)
            .expect("Failed to subscribe to BitVMX pegin events");

        info!("Successfully subscribed to BitVMX pegin events");

        let mut processor = Self {
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
            pending_pegin_requested: HashMap::new(),
            pending_all_operator_take_txids_added: HashMap::new(),
            pending_pegin_accepted: HashMap::new(),
            unconfirmed_pegin_requests: HashMap::new(),
            pegin_retry_scheduler: TickScheduler::new(),
            unconfirmed_accept_pegin: HashMap::new(),
            accept_pegin_retry_scheduler: TickScheduler::new(),
            store: Rc::clone(store),
            signaling,
            native_bridge_verifier,
            required_confirmations,
            btc_confirmations,
            btc_status_retry_blocks,
        };

        let flow_factory = |saved_state: State| {
            PeginFlow::from_saved_state(
                Rc::clone(&processor.contracts_gateway),
                processor.rt_sync.clone(),
                Rc::clone(&processor.bitvmx_broker),
                saved_state,
                Rc::clone(&processor.store),
                processor.signaling.clone(),
                processor.native_bridge_verifier.clone(),
            )
        };

        // restore_flows returns HashMap<Uuid, _> keyed by the opaque store
        // uuid; re-index by the canonical pegin id (request-pegin BTC txid).
        let restored: HashMap<Uuid, PeginFlow<CG, BC, S>> =
            restore_flows(store.as_ref(), StorePrefix::PeginFlow, flow_factory)
                .expect("Failed to load flows from store");
        processor.pegin_flows = restored.into_values().map(|flow| (flow.flow_id(), flow)).collect();
        processor
            .restore_processor_state()
            .expect("Failed to load pegin processor state from store");

        processor
    }

    /// Find a pegin flow by the `BitVMX` protocol id `BitVMX` events carry.
    /// O(n) over the in-memory map, but n is small (concurrent pegins per
    /// committee) and `BitVMX` events are infrequent.
    /// Look up by the raw `Uuid` `BitVMX` events carry. The argument is the
    /// raw uuid because `BitVMX` event payloads use the plain type; the
    /// comparison unwraps the typed protocol id internally.
    fn pegin_flow_by_protocol_id(
        &mut self,
        protocol_id: &Uuid,
    ) -> Option<&mut PeginFlow<CG, BC, S>> {
        self.pegin_flows
            .values_mut()
            .find(|flow| flow.bitvmx_protocol_id_opt().map(|p| p.value()) == Some(*protocol_id))
    }

    fn pegin_flow_by_protocol_id_ref(&self, protocol_id: &Uuid) -> Option<&PeginFlow<CG, BC, S>> {
        self.pegin_flows
            .values()
            .find(|flow| flow.bitvmx_protocol_id_opt().map(|p| p.value()) == Some(*protocol_id))
    }

    fn snapshot_processor_state(&self) -> PeginProcessorState {
        PeginProcessorState {
            events_confirming: self
                .events_confirming
                .iter()
                .filter_map(|(id, event)| event.snapshot().map(|snapshot| (id.clone(), snapshot)))
                .collect(),
            tx_status_scheduler: self.tx_status_scheduler.clone(),
            pegin_request_tracker: self.pegin_request_tracker.clone(),
            pending_pegin_requested: self.pending_pegin_requested.clone(),
            pending_all_operator_take_txids_added: self
                .pending_all_operator_take_txids_added
                .clone(),
            pending_pegin_accepted: self.pending_pegin_accepted.clone(),
            unconfirmed_pegin_requests: self.unconfirmed_pegin_requests.clone(),
            pegin_retry_scheduler: self.pegin_retry_scheduler.clone(),
            unconfirmed_accept_pegin: self.unconfirmed_accept_pegin.clone(),
            accept_pegin_retry_scheduler: self.accept_pegin_retry_scheduler.clone(),
            signature_flows: self
                .signature_flows
                .iter()
                .filter_map(|(id, flow)| flow.snapshot().map(|snapshot| (*id, snapshot)))
                .collect(),
        }
    }

    fn restore_processor_state(&mut self) -> Result<()> {
        if let Some(state) =
            self.store.load_flow::<PeginProcessorState>(&StoreKey::PeginProcessorState)?
        {
            self.apply_processor_state(state)?;
            return Ok(());
        }

        self.reconstruct_runtime_state_from_flows();
        Ok(())
    }

    fn apply_processor_state(&mut self, state: PeginProcessorState) -> Result<()> {
        let blockchain_view = BlockchainView::new();
        self.events_confirming = state
            .events_confirming
            .into_iter()
            .map(|(id, snapshot)| {
                ConfirmableEventWithData::from_snapshot(snapshot, blockchain_view.clone())
                    .map(|event| (id, event))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        self.blockchain_view = blockchain_view;
        self.tx_status_scheduler = state.tx_status_scheduler;
        self.pegin_request_tracker = state.pegin_request_tracker;
        self.pending_pegin_requested = state.pending_pegin_requested;
        self.pending_all_operator_take_txids_added = state.pending_all_operator_take_txids_added;
        self.pending_pegin_accepted = state.pending_pegin_accepted;
        self.unconfirmed_pegin_requests = state.unconfirmed_pegin_requests;
        self.pegin_retry_scheduler = state.pegin_retry_scheduler;
        self.unconfirmed_accept_pegin = state.unconfirmed_accept_pegin;
        self.accept_pegin_retry_scheduler = state.accept_pegin_retry_scheduler;
        self.signature_flows = state
            .signature_flows
            .into_iter()
            .map(|(id, snapshot)| {
                self.btc_sig_subflow_factory
                    .create_flow_from_snapshot(snapshot)
                    .map(|flow| (id, flow))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        Ok(())
    }

    fn reconstruct_runtime_state_from_flows(&mut self) {
        for (flow_id, flow) in &self.pegin_flows {
            match flow.current_step() {
                Steps::RequestPeginSpvProof => {
                    self.pegin_request_tracker.insert(flow.request_pegin_btc_tx_id());
                }
                Steps::ConfirmAcceptPeginTransaction => {
                    self.tx_status_scheduler.schedule(*flow_id, self.btc_status_retry_blocks);
                }
                Steps::AcceptPegin => {
                    self.unconfirmed_accept_pegin.insert(*flow_id, 0);
                    self.accept_pegin_retry_scheduler
                        .schedule(*flow_id, self.btc_status_retry_blocks);
                }
                _ => {}
            }
        }
    }

    fn persist_processor_state(&self) -> Result<()> {
        self.store.save_flow(&StoreKey::PeginProcessorState, self.snapshot_processor_state())
    }

    /// Handle `PeginRequested` event by finding and updating existing flow.
    fn create_flow_for_pegin_requested(&mut self, event: &PeginRequested) -> Result<()> {
        let committee_id: CommitteeId = event.committeeId.into();

        // Check if we are members of the committee
        if !self.global_context.my_committees().im_member(&committee_id) {
            debug!("Skipping PeginRequested for committee {committee_id} - not a member");
            return Ok(());
        }
        debug!(
            "Handling PeginRequested event with committee id {committee_id}, as member I should respond"
        );

        let btc_tx_id = TxIdParser::fb_32_to_txid(event.requestPeginTxid);
        let flow_id = flow_id_from_request_pegin_txid(btc_tx_id);

        // Protocol id should be unique; if it isn't, that's an upstream bug we are flagging here.
        let committee_uuid = Uuid::from_u128(*committee_id);
        let slot_index = usize::try_from(event.streamPosition.slotId)
            .map_err(|_| anyhow!("Slot ID too large for usize"))?;
        let expected_pid = accept_pegin_protocol_id(committee_uuid, slot_index);
        if let Some((other_id, _)) = self.pegin_flows.iter().find(|(fid, flow)| {
            **fid != flow_id && flow.bitvmx_protocol_id_opt() == Some(expected_pid)
        }) {
            bail!(
                "PeginFlow {flow_id}: another flow ({other_id}) already holds BitVMX protocol id {expected_pid}; refusing PeginRequested"
            );
        }

        if let Some(existing_flow) = self.pegin_flows.get_mut(&flow_id) {
            info!("Completing PeginRequested step for pegin flow {}", existing_flow.flow_id());

            let step_data = StepData::PeginRequested(event.clone());
            existing_flow.complete_step(&step_data)?;
        } else {
            warn!(
                "No existing pegin flow found for Bitcoin tx: {btc_tx_id}. This should not happen if PeginTransactionFound was processed."
            );
        }

        Ok(())
    }

    /// Handle confirmed `PeginAccepted` event.
    ///
    /// Opens a `pegin_accepted` sub-span carrying `tx_id` and
    /// `accept_pegin_tx_id`. The caller is expected to have already entered
    /// the outer `pegin{pegin_id}` span (every path lands here via the RSK
    /// dispatchers or replay helpers, which all open it).
    #[instrument(
        skip(self, pa),
        name = "pegin_accepted",
        fields(
            tx_id = %pa.inner.requestPeginTxid,
            accept_pegin_tx_id = %pa.inner.acceptPeginTxid,
        )
    )]
    fn handle_pegin_accepted(&mut self, pa: &PeginAcceptedEvent) -> Result<()> {
        info!("Processing confirmed PeginAccepted event");
        let event_accept_pegin_txid = TxIdParser::fb_32_to_txid(pa.inner.acceptPeginTxid);

        // Find the flow corresponding to this pegin acceptance using accept_pegin_tx_hash
        let flow_opt = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid_from_bitvmx_var() == Some(event_accept_pegin_txid)
        });

        if let Some(flow) = flow_opt {
            let flow_id = flow.flow_id();
            let current_step = flow.current_step();

            if current_step == Steps::Done {
                debug!("PeginAccepted already processed for flow_id={flow_id}");
                return Ok(());
            }

            if !current_step.allows_fast_forward_to_pegin_accepted() {
                warn!(
                    "Buffering PeginAccepted for acceptPeginTxid={:?} until flow_id={} reaches the accept pegin finalization checkpoint. current_step={:?}",
                    pa.inner.acceptPeginTxid, flow_id, current_step
                );
                self.pending_pegin_accepted.insert(event_accept_pegin_txid, pa.clone());
                return Ok(());
            }

            let step_data = StepData::PeginAccepted(pa.inner.clone());
            flow.complete_step(&step_data)?;
        } else {
            bail!(
                "No matching pegin flow found for PeginAccepted event: acceptPeginTxid={}. This indicates a missing or corrupted flow state.",
                pa.inner.acceptPeginTxid
            );
        }
        Ok(())
    }

    /// Process confirmed RSK events.
    ///
    /// Callers must enter a `pegin{pegin_id}` span before invoking this
    /// (the `pegin_id` is event-derived; see [`Self::pegin_id_for_event`]).
    /// Every call site — RSK dispatcher, block confirmations, and the
    /// `replay_pending_*` helpers — already opens that outer span.
    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::PeginRequested(pr) => {
                let committee_id: CommitteeId = pr.inner.committeeId.into();
                if !self.global_context.my_committees().im_member(&committee_id) {
                    debug!(
                        "Handling PeginRequested event with committee id {committee_id}, I am NOT member so I skip"
                    );
                    return Ok(());
                }
                debug!(
                    request_pegin_tx_id = %pr.inner.requestPeginTxid,
                    "Processing confirmed PeginRequested event: {pr:?}"
                );
                let btc_tx_id = TxIdParser::fb_32_to_txid(pr.inner.requestPeginTxid);
                let flow_id = flow_id_from_request_pegin_txid(btc_tx_id);
                let should_buffer = self
                    .pegin_flows
                    .get(&flow_id)
                    .is_none_or(|flow| flow.current_step() != Steps::WaitPeginRequested);

                if should_buffer {
                    info!(
                        "Buffering PeginRequested for Bitcoin tx: {btc_tx_id} until request SPV flow is ready"
                    );
                    self.pending_pegin_requested.insert(btc_tx_id, pr.clone());
                } else {
                    self.create_flow_for_pegin_requested(&pr.inner)?;
                }
            }
            RskPegManagerEvents::PeginAccepted(pa) => {
                self.handle_pegin_accepted(pa)?;
            }
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(aottah) => {
                self.handle_all_operator_take_tx_hashes_added(aottah)?;
            }
            _ => {
                trace!("Ignoring confirmed RSK event: {}", type_name_of_val(event));
            }
        }

        self.cleanup_terminal_flows();
        Ok(())
    }

    fn handle_all_operator_take_tx_hashes_added(
        &mut self,
        event: &AllOperatorTakeTxidsAddedEvent,
    ) -> Result<()> {
        let event_accept_pegin_txid = TxIdParser::fb_32_to_txid(event.inner.acceptPeginTxid);

        // Find the flow by accept_pegin_tx_hash
        let flow_with_bitvmx_accept_pegin = self.pegin_flows.values_mut().find(|flow| {
            flow.get_accept_pegin_txid_from_bitvmx_var() == Some(event_accept_pegin_txid)
        });

        if let Some(flow) = flow_with_bitvmx_accept_pegin {
            let flow_id = flow.flow_id();
            // Outer `pegin{pegin_id}` is opened by the caller (RSK dispatcher
            // via `process_confirmed_rsk_event`, or `process_new_bitvmx_event`
            // when replaying buffered events). The sub-span here only adds
            // operation context — it deliberately does not repeat pegin_id.
            let _span = info_span!("pegin_all_operator_take_txids_added").entered();
            debug!(
                "Processing AllOperatorTakeTxidsAdded: acceptPeginTxid={}",
                event.inner.acceptPeginTxid
            );
            let protocol_id = flow
                .bitvmx_protocol_id_opt()
                .ok_or_else(|| anyhow!("bitvmx_protocol_id not set for flow {flow_id}"))?;
            let protocol_uuid = protocol_id.value();

            // Start the BTC signature flow if not already started
            if self.signature_flows.contains_key(&protocol_uuid) {
                error!("BTC signature flow already started: flow_id={flow_id}");
            } else {
                info!("Starting BTC signature flow: flow_id={flow_id}");

                let pegin_accepted = flow.get_bitvmx_pegin_accepted().ok_or_else(|| {
                    anyhow!("PeginAcceptedMessage not found for flow_id: {flow_id}.")
                })?;

                let hash_to_sign =
                    Hash256::from(TxIdParser::txid_to_fb_32(event_accept_pegin_txid));
                let register_input = RegisterSignaturesBitVmxData {
                    hash_to_sign,
                    nonce: pegin_accepted.accept_pegin_nonce.clone(),
                    signature: pegin_accepted.accept_pegin_signature,
                };

                let mut btc_sig_subflow = self.btc_sig_subflow_factory.create_flow(
                    protocol_uuid,
                    flow.log_id().to_string(),
                    Some(ParentSpan::Pegin(flow_id.value())),
                );
                btc_sig_subflow.start_signature_flow(protocol_uuid, &register_input)?;

                self.signature_flows.insert(protocol_uuid, btc_sig_subflow);

                // Complete the wait step to move to the next state
                let step_data = StepData::AllOperatorTakeTxidsAdded;
                flow.complete_step(&step_data)?;
            }
        } else if self.has_flow_for_accept_pegin(event.inner.acceptPeginTxid) {
            warn!(
                "Buffering AllOperatorTakeTxidsAdded for acceptPeginTxid={:?} until BitVMX pegin_accepted arrives",
                event.inner.acceptPeginTxid
            );
            self.pending_all_operator_take_txids_added
                .insert(event_accept_pegin_txid, event.clone());
        } else {
            warn!(
                "Received AllOperatorTakeTxidsAdded: unknown_acceptPeginTxid={:?}",
                event.inner.acceptPeginTxid
            );
        }

        Ok(())
    }

    fn has_flow_for_accept_pegin(&self, accept_pegin_txid: FixedBytes<32>) -> bool {
        self.pegin_flows.values().any(|flow| {
            flow.get_state()
                .ctx
                .pegin_requested
                .as_ref()
                .map(|pegin_requested| pegin_requested.acceptPeginTxid)
                == Some(accept_pegin_txid)
        })
    }

    /// Resolve the originating pegin flow id for an RSK event, if known.
    /// `PeginRequested` derives it from the request-pegin BTC txid;
    /// `PeginAccepted` and `AllOperatorTakeTxidsAdded` look up the in-memory
    /// flow by `accept_pegin_txid`.
    fn pegin_id_for_event(&self, event: &RskPegManagerEvents) -> Option<FlowId> {
        match event {
            RskPegManagerEvents::PeginRequested(e) => {
                let btc_tx_id = TxIdParser::fb_32_to_txid(e.inner.requestPeginTxid);
                Some(flow_id_from_request_pegin_txid(btc_tx_id))
            }
            RskPegManagerEvents::PeginAccepted(e) => {
                let accept_pegin_txid = TxIdParser::fb_32_to_txid(e.inner.acceptPeginTxid);
                self.pegin_flows
                    .values()
                    .find(|flow| {
                        flow.get_accept_pegin_txid_from_bitvmx_var() == Some(accept_pegin_txid)
                    })
                    .map(PeginFlow::flow_id)
            }
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(e) => {
                let accept_pegin_txid = TxIdParser::fb_32_to_txid(e.inner.acceptPeginTxid);
                self.pegin_flows
                    .values()
                    .find(|flow| {
                        flow.get_accept_pegin_txid_from_bitvmx_var() == Some(accept_pegin_txid)
                    })
                    .map(PeginFlow::flow_id)
            }
            _ => None,
        }
    }

    /// Build event info for `PeginRequested` events
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
        event: &AllOperatorTakeTxidsAddedEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("all-operator-take-tx-hashes-added-{}", event.tx_hash),
            event.removed,
            event.block_number,
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(event.clone()),
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

        for protocol_id in &flows_to_dispatch {
            // Always remove the signature flow when it's done
            self.signature_flows.remove(protocol_id);

            let Some(flow) = self.pegin_flow_by_protocol_id_ref(protocol_id) else {
                warn!(
                    "Signature flow done for unknown pegin protocol_id: {protocol_id}. Skipping dispatch step"
                );
                continue;
            };
            let flow_id = flow.flow_id();
            let _span = info_span!("pegin", pegin_id = %flow_id).entered();

            // Only complete the step if the flow is still waiting for signatures
            if flow.current_step() != Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint {
                warn!(
                    "Signature flow completed for flow_id: {} but flow is at step {:?}, expected {:?}. Skipping dispatch step.",
                    flow.flow_id(),
                    flow.current_step(),
                    Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint
                );
                continue;
            }

            let accept_pegin_txid = flow.get_accept_pegin_txid_from_bitvmx_var();

            let Some(flow) = self.pegin_flow_by_protocol_id(protocol_id) else {
                warn!(
                    "Signature flow done for unknown pegin protocol_id: {protocol_id}. Skipping dispatch step"
                );
                continue;
            };

            let step_data = StepData::AcceptPeginSignaturesReady;
            flow.complete_step(&step_data)?;

            if let Some(accept_pegin_txid) = accept_pegin_txid {
                self.replay_pending_pegin_accepted(&accept_pegin_txid)?;
            }
        }

        Ok(())
    }

    fn handle_transaction_status_received(
        &mut self,
        protocol_id: &Uuid,
        tx_status: TransactionStatus,
    ) -> Result<()> {
        let btc_confirmations = self.btc_confirmations;
        let btc_status_retry_blocks = self.btc_status_retry_blocks;
        let Some(flow) = self.pegin_flow_by_protocol_id(protocol_id) else {
            trace!("Ignoring BitVMX Transaction event for unknown protocol_id: {protocol_id}");
            return Ok(());
        };

        let TransactionStatus { tx_id, confirmations, .. } = tx_status;
        let flow_id = flow.flow_id();
        let _span = info_span!("pegin", pegin_id = %flow_id).entered();
        let expected_txid = flow
            .get_accept_pegin_txid_from_bitvmx_var()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;

        if expected_txid != tx_id {
            return Err(anyhow!(
                "Pegin state for flow_id: {flow_id} does not match received tx_id: {tx_id:?} from tx status message"
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

        if confirmations >= btc_confirmations {
            debug!("Transaction confirmed with sufficient confirmations for flow_id: {flow_id}");
            let step_data = StepData::AcceptPeginTransactionConfirmed(tx_status);
            flow.complete_step(&step_data)?;
            if self.tx_status_scheduler.is_scheduled(&flow_id) {
                self.tx_status_scheduler.cancel(&flow_id);
            }
        } else {
            debug!(
                "Bitcoin transaction {tx_id} missing confirmations ({confirmations}/{btc_confirmations}) for flow_id {flow_id}, rescheduling"
            );
            self.tx_status_scheduler.schedule(flow_id, btc_status_retry_blocks);
        }

        Ok(())
    }

    fn handle_transaction_status_tick(&mut self) -> Result<()> {
        if self.tx_status_scheduler.is_empty() {
            return Ok(());
        }

        let ready = self.tx_status_scheduler.tick();
        for flow_id in ready {
            let _span = info_span!("pegin", pegin_id = %flow_id).entered();
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
                    warn!("Skipping delayed transaction status request for unknown flow {flow_id}");
                }
            }
        }

        Ok(())
    }

    /// Mark a pegin flow as failed and stop pending local work for it.
    ///
    /// Reachable at any step, including before Setup.
    fn fail_flow(&mut self, flow_id: FlowId, reason: &str) -> Result<()> {
        let _span = info_span!("pegin", pegin_id = %flow_id).entered();
        if let Some(flow) = self.pegin_flows.get_mut(&flow_id) {
            flow.mark_failed(reason)?;
            warn!("Admin marked pegin flow {flow_id} as failed: {reason}");
        } else {
            warn!("Admin requested fail for unknown pegin flow {flow_id}: {reason}");
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn schedule_request_pegin_retry(
        &mut self,
        spv_proof: BtcTxSPVProof,
        attempt: i16,
        reason: &str,
    ) {
        let block_hash = spv_proof.block_hash.clone();
        info!("{reason} for block {block_hash} (attempt {attempt})");
        self.unconfirmed_pegin_requests.insert(block_hash.clone(), (spv_proof, attempt));
        self.pegin_retry_scheduler.schedule(block_hash, self.btc_status_retry_blocks);
    }

    /// Callers must enter a `pegin{pegin_id}` span (or descendant) before
    /// invoking this — both `handle_spv_proof_for_accept_pegin` and
    /// `handle_pegin_retry_tick`'s accept-pegin loop already do so.
    fn schedule_accept_pegin_retry(&mut self, flow_id: FlowId, attempt: i16, reason: &str) {
        info!(attempt, "{reason}");
        self.unconfirmed_accept_pegin.insert(flow_id, attempt);
        self.accept_pegin_retry_scheduler.schedule(flow_id, self.btc_status_retry_blocks);
    }

    fn handle_pegin_retry_tick(&mut self) -> Result<()> {
        if self.pegin_retry_scheduler.is_empty() && self.accept_pegin_retry_scheduler.is_empty() {
            return Ok(());
        }

        for block_hash in self.pegin_retry_scheduler.tick() {
            debug!("(Re)trying request_pegin for block {block_hash}");

            let Some((spv_proof, attempt)) = self.unconfirmed_pegin_requests.remove(&block_hash)
            else {
                warn!("No unconfirmed pegin request found for block: {block_hash}");
                continue;
            };

            let tx_id = spv_proof.tx.compute_txid();
            let Some(flow_id) = self.request_pegin_flow_id(&tx_id) else {
                warn!("No pegin flow found for request_pegin retry: tx_id={tx_id}");
                continue;
            };
            let _span = info_span!("pegin", pegin_id = %flow_id).entered();

            let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
                warn!("No pegin flow found for request_pegin retry: flow_id={flow_id}");
                continue;
            };

            let Err(err) = flow.complete_step(&StepData::RetryRequestPegin) else {
                info!("Request pegin succeeded on retry for block {block_hash}");
                self.pegin_request_tracker.remove(&tx_id);
                self.replay_pending_pegin_requested(&tx_id)?;
                continue;
            };

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for request_pegin: {err:?}");
                self.pegin_request_tracker.remove(&tx_id);
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_request_pegin_retry(
                spv_proof,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }

        for flow_id in self.accept_pegin_retry_scheduler.tick() {
            let _span = info_span!("pegin", pegin_id = %flow_id).entered();
            let Some(attempt) = self.unconfirmed_accept_pegin.remove(&flow_id) else {
                warn!("No accept_pegin retry state found for flow {flow_id}");
                continue;
            };

            let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
                warn!("No pegin flow found for accept_pegin retry: {flow_id}");
                continue;
            };

            if flow.current_step() != Steps::AcceptPegin {
                debug!(
                    "Skipping accept_pegin retry for flow {flow_id} in step {:?}",
                    flow.current_step()
                );
                continue;
            }

            let Err(err) = flow.complete_step(&StepData::RetryAcceptPegin) else {
                info!("Accept pegin succeeded on retry for flow {flow_id}");
                continue;
            };

            if is_pegin_already_accepted(&err) {
                info!(
                    "Pegin already accepted on retry for accept_pegin for flow {flow_id}: {err:#}"
                );
                continue;
            }

            if !is_missing_native_bridge_confirmations(&err) {
                error!("Error on retry for accept_pegin: {err:?}");
                continue;
            }

            let next_attempt = attempt.saturating_add(1);
            self.schedule_accept_pegin_retry(
                flow_id,
                next_attempt,
                "Still missing confirmations on native bridge, scheduling another retry",
            );
        }

        Ok(())
    }

    fn replay_pending_pegin_requested(&mut self, tx_id: &Txid) -> Result<()> {
        if let Some(pegin_requested_event) = self.pending_pegin_requested.remove(tx_id) {
            info!("Replaying buffered PeginRequested for Bitcoin tx: {tx_id}");
            self.process_confirmed_rsk_event(&RskPegManagerEvents::PeginRequested(
                pegin_requested_event,
            ))?;
        }

        Ok(())
    }

    fn replay_pending_all_operator_take_txids_added(
        &mut self,
        accept_pegin_txid: &Txid,
    ) -> Result<()> {
        let current_step = self
            .pegin_flows
            .values()
            .find(|flow| flow.get_accept_pegin_txid_from_bitvmx_var() == Some(*accept_pegin_txid))
            .map(PeginFlow::current_step);

        if current_step != Some(Steps::WaitAllOperatorTakeTxidsAdded) {
            debug!(
                "Keeping buffered AllOperatorTakeTxidsAdded for accept pegin tx: {accept_pegin_txid}; current_step={current_step:?}"
            );
            return Ok(());
        }

        if let Some(event) = self.pending_all_operator_take_txids_added.remove(accept_pegin_txid) {
            info!(
                "Replaying buffered AllOperatorTakeTxidsAdded for accept pegin tx: {accept_pegin_txid}"
            );
            self.process_confirmed_rsk_event(&RskPegManagerEvents::AllOperatorTakeTxidsAdded(
                event,
            ))?;
        }

        Ok(())
    }

    fn replay_pending_pegin_accepted(&mut self, accept_pegin_txid: &Txid) -> Result<()> {
        if let Some(event) = self.pending_pegin_accepted.remove(accept_pegin_txid) {
            info!("Replaying buffered PeginAccepted for accept pegin tx: {accept_pegin_txid}");
            self.process_confirmed_rsk_event(&RskPegManagerEvents::PeginAccepted(event))?;
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
                let pegin_id = self.pegin_id_for_event(event.get_data());
                let _span = pegin_id.map(|fid| info_span!("pegin", pegin_id = %fid).entered());
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        self.cleanup_terminal_flows();

        Ok(())
    }

    fn subscribe_to_bitvmx_pegin_events(bitvmx_broker: &BC, confirmations: u32) -> Result<()> {
        bitvmx_broker.send(IncomingBitVMXApiMessages::SubscribeToRskPegin(Some(confirmations)))?;

        Ok(())
    }

    fn handle_pegin_transaction_found(&mut self, tx_id: Txid) -> Result<()> {
        let flow_id = flow_id_from_request_pegin_txid(tx_id);
        let _span = info_span!("pegin", pegin_id = %flow_id, tx_id = %tx_id).entered();
        if self.pegin_request_tracker.contains(&tx_id) || self.pegin_flows.contains_key(&flow_id) {
            debug!("Ignoring duplicate BitVMX pegin event for tx_id={tx_id}");
            return Ok(());
        }

        let mut flow = PeginFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            tx_id,
            flow_id,
            Rc::clone(&self.store),
            self.signaling.clone(),
            self.native_bridge_verifier.clone(),
        );

        info!("Created new pegin flow {flow_id} from Bitcoin transaction: {tx_id}");

        let step_data = StepData::PeginTransactionFound;
        flow.complete_step(&step_data)?;

        self.pegin_flows.insert(flow_id, flow);
        self.pegin_request_tracker.insert(tx_id);

        Ok(())
    }

    /// Look up a pegin flow that's currently in
    /// `RequestPeginSpvProof`. The argument is the request-pegin BTC txid,
    /// which is also the flow's `HashMap` key — so this is just a guarded
    /// `get`.
    fn request_pegin_flow_id(&self, tx_id: &Txid) -> Option<FlowId> {
        let flow_id = flow_id_from_request_pegin_txid(*tx_id);
        let flow = self.pegin_flows.get(&flow_id)?;
        (flow.current_step() == Steps::RequestPeginSpvProof).then_some(flow_id)
    }

    #[instrument(
        name = "pegin",
        skip(self, spv_proof),
        fields(tx_id = %tx_id, pegin_id = tracing::field::Empty),
    )]
    fn handle_spv_proof_for_request_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: BtcTxSPVProof,
    ) -> Result<()> {
        if !self.pegin_request_tracker.contains(tx_id) {
            return Ok(()); // Not a tracked pegin request
        }

        info!("Handling request pegin SPV proof: tx_id={tx_id}");

        let Some(flow_id) = self.request_pegin_flow_id(tx_id) else {
            warn!("No pegin flow found for request_pegin: tx_id={tx_id}");
            return Ok(());
        };
        record_pegin_id(&flow_id);

        let Some(flow) = self.pegin_flows.get_mut(&flow_id) else {
            warn!("No pegin flow found for request_pegin: flow_id={flow_id}");
            return Ok(());
        };

        let step_data = StepData::RequestPeginSpvProof(spv_proof.clone());
        if let Err(err) = flow.complete_step(&step_data) {
            if is_missing_native_bridge_confirmations(&err) {
                self.schedule_request_pegin_retry(
                    spv_proof,
                    1,
                    "Missing confirmations on native bridge, scheduling retry",
                );
                return Ok(());
            }

            return Err(err);
        }

        self.pegin_request_tracker.remove(tx_id);
        debug!("Removed request_pegin_txid from tracking: tx_id={tx_id}");

        self.replay_pending_pegin_requested(tx_id)
    }

    fn handle_spv_proof_for_accept_pegin(
        &mut self,
        tx_id: &Txid,
        spv_proof: BtcTxSPVProof,
    ) -> Result<()> {
        // Find state by matching accept_pegin_txid from bitvmx_pegin_accepted
        let flow_opt = self
            .pegin_flows
            .values_mut()
            .find(|flow| flow.get_accept_pegin_txid_from_bitvmx_var() == Some(*tx_id));

        if let Some(flow) = flow_opt {
            let flow_id = flow.flow_id();
            let _span = info_span!("pegin", pegin_id = %flow_id, tx_id = %tx_id).entered();
            info!("Handling accept pegin SPV proof");
            let step_data = StepData::AcceptPeginSpvProof(spv_proof);
            if let Err(err) = flow.complete_step(&step_data) {
                if is_missing_native_bridge_confirmations(&err) {
                    let attempt = self
                        .unconfirmed_accept_pegin
                        .get(&flow_id)
                        .copied()
                        .unwrap_or(0)
                        .saturating_add(1);
                    self.schedule_accept_pegin_retry(
                        flow_id,
                        attempt,
                        "Missing confirmations on native bridge, scheduling retry",
                    );
                    return Ok(());
                }
                if is_pegin_already_accepted(&err) {
                    info!("Pegin already accepted on accept_pegin for flow {flow_id}: {err:#}");
                    return Ok(());
                }
                return Err(err);
            }
        }

        Ok(())
    }

    fn has_flow_waiting_for_accept_pegin_spv(&self, tx_id: &Txid) -> bool {
        self.pegin_flows
            .values()
            .any(|flow| flow.get_accept_pegin_txid_from_bitvmx_var() == Some(*tx_id))
    }

    fn cleanup_terminal_flow_state(&mut self) {
        let terminal: Vec<(FlowId, Option<BitVmxProtocolId>, Txid)> = self
            .pegin_flows
            .values()
            .filter(|flow| flow.is_terminal())
            .map(|flow| {
                (flow.flow_id(), flow.bitvmx_protocol_id_opt(), flow.request_pegin_btc_tx_id())
            })
            .collect();

        for (flow_id, protocol_id, request_pegin_btc_tx_id) in terminal {
            if let Some(protocol_id) = protocol_id {
                self.signature_flows.remove(&protocol_id.value());
            }
            self.tx_status_scheduler.cancel(&flow_id);
            self.accept_pegin_retry_scheduler.cancel(&flow_id);
            self.unconfirmed_accept_pegin.remove(&flow_id);

            self.pegin_request_tracker.remove(&request_pegin_btc_tx_id);
            self.pending_pegin_requested.remove(&request_pegin_btc_tx_id);

            let retry_keys: Vec<_> = self
                .unconfirmed_pegin_requests
                .iter()
                .filter(|(_, (spv_proof, _))| {
                    spv_proof.tx.compute_txid() == request_pegin_btc_tx_id
                })
                .map(|(key, _)| key.clone())
                .collect();

            for key in retry_keys {
                self.pegin_retry_scheduler.cancel(&key);
                self.unconfirmed_pegin_requests.remove(&key);
            }
        }
    }

    fn cleanup_terminal_flows(&mut self) {
        self.cleanup_terminal_flow_state();
        // FlowId is a Uuid derived from the request-pegin txid and is also the
        // store key. Delete terminal entries directly via the flow id.
        let terminal_flow_ids: Vec<FlowId> = self
            .pegin_flows
            .values()
            .filter(|flow| flow.is_terminal())
            .map(PeginFlow::flow_id)
            .collect();
        for flow_id in terminal_flow_ids {
            if let Err(err) = self.store.delete_flow(&StoreKey::PeginFlow(flow_id.value())) {
                error!("Failed to remove pegin flow {flow_id} from persistence: {err}");
            }
        }
        // Drop terminal entries from the in-memory map.
        self.pegin_flows.retain(|_, flow| !flow.is_terminal());
    }
}

impl<CG, BC, S> EventProcessor
    for PeginFlowProcessor<
        CG,
        BC,
        BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>,
        BtcSignatureSubFlowFactory<CG>,
        S,
    >
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        self.cleanup_terminal_flows();

        // Pegin flows are created from RSK / BTC events, not user requests, except for
        // the admin "fail flow" lever — handled here so cleanup runs alongside this
        // processor's in-memory state.
        if let UserRequests::Admin(AdminRequest::FailFlow { kind, flow_id, reason }) = req
            && *kind == FlowKind::Pegin
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
            OutgoingBitVMXApiMessages::OutputPatternTransactionFound(tx_id, _tx_status, tag) => {
                if tag.as_slice() == RSK_PEGIN_TAG {
                    debug!(
                        "Received BitVMX OutputPatternTransactionFound for RSK pegin: tx_id={tx_id}"
                    );
                } else {
                    trace!(
                        "Ignoring BitVMX OutputPatternTransactionFound with non-pegin tag: {:?}",
                        String::from_utf8_lossy(tag)
                    );
                }
            }
            // Handle PeginTransactionFound from BitVMX
            OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, _tx_status) => {
                debug!("Received BitVMX PeginTransactionFound: tx_id={tx_id}");
                self.handle_pegin_transaction_found(*tx_id)?;
            }
            // Handle SPVProof from BitVMX (for both request and accept pegin)
            OutgoingBitVMXApiMessages::SPVProof(tx_id, spv_proof_opt) => {
                if let Some(spv_proof) = spv_proof_opt {
                    debug!("Received BitVMX SPVProof: tx_id={tx_id}");
                    trace!("Received spv_proof_data for tx_id={tx_id}: {spv_proof:?}");

                    // Route SPV proof to the appropriate handler based on context
                    if self.pegin_request_tracker.contains(tx_id) {
                        // Handle as request pegin SPV proof
                        self.handle_spv_proof_for_request_pegin(tx_id, spv_proof.clone())?;
                    } else if self.has_flow_waiting_for_accept_pegin_spv(tx_id) {
                        // Handle as accept pegin SPV proof
                        self.handle_spv_proof_for_accept_pegin(tx_id, spv_proof.clone())?;
                    } else {
                        debug!(
                            "SPV proof for tx_id: {tx_id} does not match any tracked pegin request or flow"
                        );
                    }
                } else {
                    return Err(anyhow!(
                        "Received BitVMX SPVProof event for tx_id: {tx_id}, but no SPV proof was included."
                    ));
                }
            }
            // Handle CommInfo from BitVMX
            OutgoingBitVMXApiMessages::CommInfo(req_id, comm_info) => {
                trace!("Received CommInfo from BitVMX req_id: {req_id}, comm_info: {comm_info:?}");
                // For any flow in GetCommInfoAuthoritativeCheckpoint step, complete the step with the CommInfo
                for (flow_id, flow) in &mut self.pegin_flows {
                    if flow.current_step() == Steps::GetCommInfoAuthoritativeCheckpoint {
                        let _span = info_span!("pegin", pegin_id = %flow_id, tx_id = %flow.get_state().ctx.request_pegin_btc_tx_id).entered();
                        debug!(
                            "Completing GetCommInfoAuthoritativeCheckpoint step for flow {flow_id}"
                        );
                        let step_data = StepData::CommInfo(comm_info.clone());
                        flow.complete_step(&step_data)?;
                    }
                }
            }
            // Handle PeginAccepted variable from BitVMX
            OutgoingBitVMXApiMessages::Variable(
                protocol_id,
                method,
                VariableTypes::String(data),
            ) if matches!(method.as_str(), PEGIN_ACCEPTED_INPUT_MSG) => {
                let flow_id_opt =
                    self.pegin_flow_by_protocol_id_ref(protocol_id).map(PeginFlow::flow_id);
                let _span = flow_id_opt.map(|fid| info_span!("pegin", pegin_id = %fid).entered());
                info!("Received PeginAccepted variable from BitVMX for protocol_id: {protocol_id}");
                debug!("PeginAccepted data: {data}");
                let pegin_accepted: PeginAcceptedMessage = serde_json::from_str(data)?;
                let accept_pegin_txid = {
                    let flow = self
                        .pegin_flow_by_protocol_id(protocol_id)
                        .ok_or_else(|| anyhow!("Flow not found for protocol_id: {protocol_id}"))?;

                    if flow.current_step() != Steps::PreparePeginSetup {
                        return Err(anyhow!(
                            "Mismatch current step for flow {} expected {:?} having {:?}",
                            flow.flow_id(),
                            Steps::PreparePeginSetup,
                            flow.current_step()
                        ));
                    }

                    let step_data = StepData::BitvmxPeginAccepted(pegin_accepted);
                    flow.complete_step(&step_data)?;
                    flow.get_accept_pegin_txid_from_bitvmx_var()
                };

                if let Some(accept_pegin_txid) = accept_pegin_txid {
                    self.replay_pending_all_operator_take_txids_added(&accept_pegin_txid)?;
                }
            }
            OutgoingBitVMXApiMessages::TransactionInfo(protocol_id, tx_name, transaction) => {
                let txid = transaction.compute_txid();
                let Some(flow) = self.pegin_flow_by_protocol_id(protocol_id) else {
                    trace!(
                        "Ignoring BitVMX TransactionInfo for unknown protocol_id: {protocol_id}"
                    );
                    return Ok(());
                };

                let flow_id = flow.flow_id();
                let _span = info_span!("pegin", pegin_id = %flow_id).entered();
                debug!(
                    "Received BitVMX TransactionInfo for flow_id: {flow_id}, tx_name: {tx_name}, txid: {txid}"
                );

                match flow.current_step() {
                    Steps::RequestOperatorTakeTransactionInfo
                        if tx_name.starts_with(OPERATOR_TAKE_TX_PREFIX) =>
                    {
                        flow.complete_step(&StepData::TransactionInfo {
                            tx_name: tx_name.clone(),
                            txid,
                        })?;
                    }
                    Steps::RequestOperatorWonTransactionInfo
                        if tx_name.starts_with(OPERATOR_WON_TX_PREFIX) =>
                    {
                        flow.complete_step(&StepData::TransactionInfo {
                            tx_name: tx_name.clone(),
                            txid,
                        })?;
                    }
                    _ => {
                        trace!(
                            "Ignoring BitVMX TransactionInfo for flow {} in step {:?}",
                            flow_id,
                            flow.current_step()
                        );
                    }
                }
            }
            // Handle SetupCompleted from BitVMX
            OutgoingBitVMXApiMessages::SetupCompleted(program_id) => {
                if let Some(flow) = self.pegin_flow_by_protocol_id_ref(program_id) {
                    let _span = info_span!("pegin", pegin_id = %flow.flow_id()).entered();
                    info!("Pegin setup was completed: protocol_id={program_id}");
                } else {
                    trace!("Ignoring BitVMX SetupCompleted for unknown program_id: {program_id}");
                }
            }
            // Handle Transaction status from BitVMX
            OutgoingBitVMXApiMessages::Transaction(flow_id, tx_status, _tx_opt) => {
                self.handle_transaction_status_received(flow_id, tx_status.clone())?;
            }
            _ => {
                trace!("Ignoring BitVMX event: {event:?}");
            }
        }

        self.persist_processor_state()
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
                return self.persist_processor_state();
            }
            _ => {
                // Continue with the normal flow
            }
        }

        // Open the outer `pegin{pegin_id}` span once for this event so both
        // the `required_confirmations == 0` direct-processing branch and the
        // confirmation-registration branch run under it. Sub-handlers must
        // not open their own `pegin` span on top.
        let _span = self
            .pegin_id_for_event(event)
            .map(|fid| info_span!("pegin", pegin_id = %fid).entered());

        // useful for testing purposes
        if self.required_confirmations == 0 {
            self.process_confirmed_rsk_event(event)?;
            return self.persist_processor_state();
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::PeginRequested(e) => Self::build_pegin_requested_event_info(e),
            RskPegManagerEvents::PeginAccepted(e) => Self::build_pegin_accepted_event_info(e),
            RskPegManagerEvents::AllOperatorTakeTxidsAdded(e) => {
                Self::build_all_operator_take_tx_hashes_added_event_info(e)
            }
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

        self.persist_processor_state()
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        self.cleanup_terminal_flows();

        self.process_unhandled_confirmed_sig_flow_events(block)?;
        self.handle_transaction_status_tick()?;
        self.handle_pegin_retry_tick()?;
        self.process_block_confirmations(block)?;

        self.persist_processor_state()
    }

    fn shutdown(&mut self) {
        info!("Shutting down PeginFlowProcessor");
        self.pegin_flows.clear();
        self.events_confirming.clear();
        self.blockchain_view.clear();
        self.tx_status_scheduler.clear();
        self.pegin_retry_scheduler.clear();
        self.unconfirmed_pegin_requests.clear();
        self.accept_pegin_retry_scheduler.clear();
        self.unconfirmed_accept_pegin.clear();
        self.pegin_request_tracker.clear();
        self.pending_pegin_requested.clear();
        self.pending_all_operator_take_txids_added.clear();
        self.pending_pegin_accepted.clear();
        self.signature_flows.clear();
    }

    fn active_flows(&self) -> Vec<crate::event_processor::FlowDetails> {
        self.pegin_flows
            .values()
            .filter(|f| !f.is_terminal())
            .map(PeginFlow::get_flow_details)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use alloy_primitives::{Address as AlloyAddress, Bytes, FixedBytes, I256, U256};
    use bitcoin::Transaction;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use common::msg_broker::bitvmx_types::{
        IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, ParticipantRole,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::test_utils::rsk_block_generator::FakeBlockGenerator;
    use common::types::{BlockHash, TxHash, TxIdParser};
    use musig2::PubNonce;
    use musig2::secp::MaybeScalar;
    use primitive_types::H256;
    use transaction_dispatcher::types::GetCommitteeOutput;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::Committee;
    use union_contracts::bindings::pegin_manager::PeginManager::{
        PeginAccepted, PeginRequested, RequestPeginTempInfo, StreamPosition,
    };
    use union_contracts::bindings::signature_manager::SignatureManager::AllOperatorTakeTxidsAdded;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::pegin::pegin_flow::FlowContext;
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

    type TestProcessor = PeginFlowProcessor<
        MockRskContractsGatewayApi,
        MockBitVmxBroker,
        TestBtcSigSubFlow,
        TestBtcSigFactory,
        MockCoordinatorStoreApi,
    >;

    type TestPeginFlow =
        PeginFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>;

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
            let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");

            let mut broker = MockBitVmxBroker::new();
            broker.expect_send().returning(|_| Ok(true));
            let broker = Rc::new(broker);

            let mut store = MockCoordinatorStoreApi::new();
            store.expect_save_flow::<State>().returning(|_, _| Ok(()));
            store.expect_save_flow::<PeginProcessorState>().returning(|_, _| Ok(()));
            store.expect_delete_flow().returning(|_| Ok(()));
            let store = Rc::new(store);

            let processor = PeginFlowProcessor {
                contracts_gateway: Rc::clone(&contracts),
                rt_sync: rt_sync.clone(),
                bitvmx_broker: Rc::clone(&broker),
                btc_sig_subflow_factory: TestBtcSigFactory::new(
                    Rc::clone(&contracts),
                    rt_sync.clone(),
                    5,
                ),
                pegin_flows: HashMap::new(),
                signature_flows: HashMap::new(),
                global_context: GlobalContext::new(),
                blockchain_view: BlockchainView::new(),
                events_confirming: HashMap::new(),
                tx_status_scheduler: TickScheduler::new(),
                pegin_request_tracker: HashSet::new(),
                pending_pegin_requested: HashMap::new(),
                pending_all_operator_take_txids_added: HashMap::new(),
                pending_pegin_accepted: HashMap::new(),
                unconfirmed_pegin_requests: HashMap::new(),
                pegin_retry_scheduler: TickScheduler::new(),
                unconfirmed_accept_pegin: HashMap::new(),
                accept_pegin_retry_scheduler: TickScheduler::new(),
                store: Rc::clone(&store),
                signaling: Rc::new(Signaling::new("/tmp", "disabled")),
                native_bridge_verifier: NativeBridgeVerifier::Dummy,
                required_confirmations: 5,
                btc_confirmations: 5,
                btc_status_retry_blocks: 1,
            };

            Self { processor, contracts, broker, store, rt_sync }
        }

        fn create_flow_at_step(
            &self,
            flow_id: FlowId,
            step: Steps,
            accept_pegin_txid: Txid,
        ) -> TestPeginFlow {
            self.create_flow_at_step_with_bitvmx_pegin_accepted(
                flow_id,
                step,
                accept_pegin_txid,
                Some(test_pegin_accepted_message(accept_pegin_txid)),
            )
        }

        fn create_flow_at_step_with_bitvmx_pegin_accepted(
            &self,
            flow_id: FlowId,
            step: Steps,
            accept_pegin_txid: Txid,
            bitvmx_pegin_accepted: Option<PeginAcceptedMessage>,
        ) -> TestPeginFlow {
            let ctx = FlowContext {
                flow_id,
                request_pegin_btc_tx_id: test_txid([1u8; 32]),
                step,
                bitvmx_protocol_id: Some(
                    common::msg_broker::bitvmx_types::accept_pegin_protocol_id(Uuid::nil(), 0),
                ),
                request_pegin_btc_tx_status: None,
                request_pegin_spv_proof: None,
                pegin_requested: Some(test_pegin_requested(accept_pegin_txid)),
                my_p2p_address: None,
                committee_output: Some(GetCommitteeOutput {
                    committee: Committee {
                        members: vec![],
                        leaderAddress: AlloyAddress::from([0u8; 20]),
                        operatorTakeIndex: U256::ZERO,
                        createdAt: U256::ZERO,
                        missingData: 0,
                        missingCommunicationData: 0,
                        isPending: false,
                        streamId: 0,
                        fundingUTXOs: vec![],
                        aggregatedKey: vec![].into(),
                    },
                }),
                bitvmx_pegin_accepted,
                operator_take_txid: None,
                operator_won_txid: None,
                accept_pegin_spv_proof: None,
                accept_pegin_tx_status: None,
                pegin_accepted: None,
                op_role: Some(ParticipantRole::Prover),
            };

            PeginFlow::from_saved_state(
                Rc::clone(&self.contracts),
                self.rt_sync.clone(),
                Rc::clone(&self.broker),
                State { flow_id, log_id: String::new(), ctx, created_at: None },
                Rc::clone(&self.store),
                Rc::new(Signaling::new("/tmp", "disabled")),
                NativeBridgeVerifier::Dummy,
            )
        }

        fn create_completed_sig_flow(&self, flow_id: Uuid) -> TestBtcSigSubFlow {
            BaseBtcSignatureSubFlow::new_completed_for_test(
                &self.contracts,
                &self.rt_sync,
                flow_id,
                String::new(),
                5,
            )
        }
    }

    fn test_txid(bytes: [u8; 32]) -> Txid {
        TxIdParser::fb_32_to_txid(FixedBytes::from(bytes))
    }

    fn test_block() -> RskBlockAndUncles {
        let block_generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        RskBlockAndUncles::new_no_uncles(
            block_generator
                .generate_block(BlockNumber::from(100), None)
                .expect("failed to generate test block"),
        )
    }

    fn test_spv_proof() -> BtcTxSPVProof {
        BtcTxSPVProof {
            block_hash: "11".repeat(32),
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

    fn default_pub_nonce() -> PubNonce {
        "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93"
            .parse::<PubNonce>()
            .expect("invalid pub nonce")
    }

    fn test_pegin_accepted_message(accept_pegin_txid: Txid) -> PeginAcceptedMessage {
        PeginAcceptedMessage {
            accept_pegin_txid,
            accept_pegin_nonce: default_pub_nonce(),
            accept_pegin_signature: MaybeScalar::Zero,
            accept_pegin_sighash: vec![],
            operator_take_sighash: Some(vec![1u8; 32]),
            operator_won_sighash: Some(vec![2u8; 32]),
            committee_id: Uuid::new_v4(),
        }
    }

    fn stream_position() -> StreamPosition {
        StreamPosition { streamId: 0, packetNumber: 0, slotId: 0, pegStatus: 0 }
    }

    fn test_pegin_requested(accept_pegin_txid: Txid) -> PeginRequested {
        PeginRequested {
            committeeId: 1,
            requestPeginTxid: FixedBytes::from([1u8; 32]),
            acceptPeginTxid: TxIdParser::txid_to_fb_32(accept_pegin_txid),
            streamPosition: stream_position(),
            requestPeginInfo: RequestPeginTempInfo {
                rskDestinationAddress: AlloyAddress::from([2u8; 20]),
                btcReimbursementPubKey: FixedBytes::from([3u8; 32]),
                acceptPeginSignatureHash: FixedBytes::from([4u8; 32]),
                btcBlockNumber: I256::ZERO,
                userReimbursementTxid: FixedBytes::ZERO,
                rejectPeginTxid: FixedBytes::ZERO,
            },
            acceptPeginSignatureMessage: Bytes::from(vec![5u8; 32]),
        }
    }

    fn test_pegin_requested_event(tx_id: Txid) -> PeginRequestedEvent {
        PeginRequestedEvent {
            inner: PeginRequested {
                committeeId: 1,
                requestPeginTxid: TxIdParser::txid_to_fb_32(tx_id),
                acceptPeginTxid: FixedBytes::<32>::ZERO,
                streamPosition: stream_position(),
                requestPeginInfo: RequestPeginTempInfo {
                    rskDestinationAddress: AlloyAddress::ZERO,
                    btcReimbursementPubKey: FixedBytes::<32>::ZERO,
                    acceptPeginSignatureHash: FixedBytes::<32>::ZERO,
                    btcBlockNumber: I256::ZERO,
                    userReimbursementTxid: FixedBytes::<32>::ZERO,
                    rejectPeginTxid: FixedBytes::<32>::ZERO,
                },
                acceptPeginSignatureMessage: Bytes::new(),
            },
            block_number: BlockNumber::from(1),
            block_hash: BlockHash::from(H256::from_low_u64_be(2)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(3)),
        }
    }

    fn test_pegin_accepted_event(accept_pegin_txid: Txid) -> PeginAcceptedEvent {
        PeginAcceptedEvent {
            inner: PeginAccepted {
                blockHash: FixedBytes::from([6u8; 32]),
                acceptPeginTxid: TxIdParser::txid_to_fb_32(accept_pegin_txid),
                requestPeginTxid: FixedBytes::from([1u8; 32]),
                vout: 0,
                streamPosition: stream_position(),
                speedUpPubKey: FixedBytes::from([7u8; 32]),
                rskDestinationAddress: AlloyAddress::from([8u8; 20]),
                rbtcAmount: U256::from(1),
                utxoScriptPubKey: Bytes::from(vec![9u8]),
            },
            block_number: BlockNumber::from(50),
            block_hash: BlockHash::from(H256::from_low_u64_be(51)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(52)),
        }
    }

    fn test_all_operator_take_txids_added_event(
        accept_pegin_txid: Txid,
    ) -> AllOperatorTakeTxidsAddedEvent {
        AllOperatorTakeTxidsAddedEvent {
            inner: AllOperatorTakeTxidsAdded {
                acceptPeginTxid: TxIdParser::txid_to_fb_32(accept_pegin_txid),
            },
            block_number: BlockNumber::from(60),
            block_hash: BlockHash::from(H256::from_low_u64_be(61)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(62)),
        }
    }

    #[test]
    fn processor_state_snapshot_restores_pegin_runtime_context() {
        let mut harness = TestHarness::new();
        let request_txid = test_txid([9u8; 32]);
        let accept_pegin_txid = test_txid([8u8; 32]);
        let flow_id = flow_id_from_request_pegin_txid(request_txid);
        let retry_key = "request-pegin-retry-key".to_string();
        let protocol_id = Uuid::new_v4();

        let mut confirmable_event = ConfirmableEventWithData::new(
            "pegin-requested-test".to_string(),
            5,
            harness.processor.blockchain_view.clone(),
            RskPegManagerEvents::PeginRequested(test_pegin_requested_event(request_txid)),
        );
        confirmable_event.start_confirming(BlockNumber::from(1)).unwrap();
        harness.processor.events_confirming.insert(confirmable_event.id(), confirmable_event);
        harness.processor.tx_status_scheduler.schedule(flow_id, 7);
        harness.processor.pegin_request_tracker.insert(request_txid);
        harness
            .processor
            .pending_pegin_requested
            .insert(request_txid, test_pegin_requested_event(request_txid));
        harness
            .processor
            .pending_all_operator_take_txids_added
            .insert(accept_pegin_txid, test_all_operator_take_txids_added_event(accept_pegin_txid));
        harness
            .processor
            .pending_pegin_accepted
            .insert(accept_pegin_txid, test_pegin_accepted_event(accept_pegin_txid));
        harness
            .processor
            .unconfirmed_pegin_requests
            .insert(retry_key.clone(), (test_spv_proof(), 3));
        harness.processor.pegin_retry_scheduler.schedule(retry_key.clone(), 11);
        harness.processor.unconfirmed_accept_pegin.insert(flow_id, 4);
        harness.processor.accept_pegin_retry_scheduler.schedule(flow_id, 13);
        harness
            .processor
            .signature_flows
            .insert(protocol_id, harness.create_completed_sig_flow(protocol_id));

        let snapshot = harness.processor.snapshot_processor_state();
        let mut restored = TestHarness::new().processor;
        restored.apply_processor_state(snapshot).unwrap();

        assert_eq!(restored.events_confirming.len(), 1);
        restored.blockchain_view.update(&test_block());
        assert!(restored.events_confirming.values().next().unwrap().is_confirmed());
        assert!(restored.tx_status_scheduler.is_scheduled(&flow_id));
        assert!(restored.pegin_request_tracker.contains(&request_txid));
        assert!(restored.pending_pegin_requested.contains_key(&request_txid));
        assert!(restored.pending_all_operator_take_txids_added.contains_key(&accept_pegin_txid));
        assert!(restored.pending_pegin_accepted.contains_key(&accept_pegin_txid));
        assert_eq!(restored.unconfirmed_pegin_requests.get(&retry_key).unwrap().1, 3);
        assert!(restored.pegin_retry_scheduler.is_scheduled(&retry_key));
        assert_eq!(restored.unconfirmed_accept_pegin.get(&flow_id), Some(&4));
        assert!(restored.accept_pegin_retry_scheduler.is_scheduled(&flow_id));
        assert!(restored.signature_flows.get(&protocol_id).unwrap().is_done());
    }

    #[test]
    fn cleanup_terminal_flows_removes_pegin_flow_and_request_side_state() {
        let mut harness = TestHarness::new();

        let spv_proof = test_spv_proof();
        let request_tx_id = spv_proof.tx.compute_txid();
        let flow_id = flow_id_from_request_pegin_txid(request_tx_id);

        let protocol_id =
            common::msg_broker::bitvmx_types::accept_pegin_protocol_id(Uuid::nil(), 0);
        let state = State {
            flow_id,
            log_id: String::new(),
            ctx: FlowContext {
                flow_id,
                request_pegin_btc_tx_id: request_tx_id,
                step: Steps::Failed,
                bitvmx_protocol_id: Some(protocol_id),
                request_pegin_btc_tx_status: None,
                request_pegin_spv_proof: None,
                pegin_requested: None,
                my_p2p_address: None,
                committee_output: None,
                bitvmx_pegin_accepted: None,
                operator_take_txid: None,
                operator_won_txid: None,
                accept_pegin_spv_proof: None,
                accept_pegin_tx_status: None,
                pegin_accepted: None,
                op_role: None,
            },
            created_at: None,
        };

        let flow = PeginFlow::from_saved_state(
            Rc::clone(&harness.contracts),
            harness.rt_sync.clone(),
            Rc::clone(&harness.broker),
            state,
            Rc::clone(&harness.store),
            Rc::new(Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        harness.processor.pegin_flows.insert(flow_id, flow);
        harness.processor.tx_status_scheduler.schedule(flow_id, 1);
        harness.processor.accept_pegin_retry_scheduler.schedule(flow_id, 1);
        harness.processor.unconfirmed_accept_pegin.insert(flow_id, 1);
        harness.processor.pegin_request_tracker.insert(request_tx_id);
        harness
            .processor
            .pending_pegin_requested
            .insert(request_tx_id, test_pegin_requested_event(request_tx_id));
        harness
            .processor
            .unconfirmed_pegin_requests
            .insert(spv_proof.block_hash.clone(), (spv_proof.clone(), 1));
        harness.processor.pegin_retry_scheduler.schedule(spv_proof.block_hash.clone(), 1);

        harness.processor.cleanup_terminal_flows();

        assert!(!harness.processor.pegin_flows.contains_key(&flow_id));
        assert!(!harness.processor.tx_status_scheduler.is_scheduled(&flow_id));
        assert!(!harness.processor.accept_pegin_retry_scheduler.is_scheduled(&flow_id));
        assert!(!harness.processor.unconfirmed_accept_pegin.contains_key(&flow_id));
        assert!(!harness.processor.pegin_request_tracker.contains(&request_tx_id));
        assert!(!harness.processor.pending_pegin_requested.contains_key(&request_tx_id));
        assert!(!harness.processor.unconfirmed_pegin_requests.contains_key(&spv_proof.block_hash));
        assert!(!harness.processor.pegin_retry_scheduler.is_scheduled(&spv_proof.block_hash));
    }

    #[test]
    fn signature_completion_advances_before_replaying_buffered_pegin_accepted() {
        let mut harness = TestHarness::new();
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let accept_pegin_txid = test_txid([8u8; 32]);

        let flow = harness.create_flow_at_step(
            flow_id,
            Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint,
            accept_pegin_txid,
        );
        let protocol_id =
            common::msg_broker::bitvmx_types::accept_pegin_protocol_id(Uuid::nil(), 0);
        harness.processor.pegin_flows.insert(flow_id, flow);
        harness
            .processor
            .pending_pegin_accepted
            .insert(accept_pegin_txid, test_pegin_accepted_event(accept_pegin_txid));
        let protocol_uuid = protocol_id.value();
        harness
            .processor
            .signature_flows
            .insert(protocol_uuid, harness.create_completed_sig_flow(protocol_uuid));

        let result = harness.processor.process_unhandled_confirmed_sig_flow_events(&test_block());

        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        assert!(!harness.processor.pegin_flows.contains_key(&flow_id));
        assert!(!harness.processor.signature_flows.contains_key(&protocol_uuid));
        assert!(!harness.processor.pending_pegin_accepted.contains_key(&accept_pegin_txid));
    }

    #[test]
    fn all_operator_take_txids_added_stays_buffered_until_bitvmx_pegin_accepted() {
        let mut harness = TestHarness::new();
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let accept_pegin_txid = test_txid([9u8; 32]);

        let flow = harness.create_flow_at_step_with_bitvmx_pegin_accepted(
            flow_id,
            Steps::PreparePeginSetup,
            accept_pegin_txid,
            None,
        );
        harness.processor.pegin_flows.insert(flow_id, flow);

        let event = test_all_operator_take_txids_added_event(accept_pegin_txid);
        let result = harness.processor.handle_all_operator_take_tx_hashes_added(&event);

        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        assert!(
            harness
                .processor
                .pending_all_operator_take_txids_added
                .contains_key(&accept_pegin_txid)
        );
        assert!(harness.processor.signature_flows.is_empty());
        assert_eq!(
            harness.processor.pegin_flows[&flow_id].current_step(),
            Steps::PreparePeginSetup
        );

        harness
            .processor
            .replay_pending_all_operator_take_txids_added(&accept_pegin_txid)
            .expect("replay should remain deferred");
        assert!(
            harness
                .processor
                .pending_all_operator_take_txids_added
                .contains_key(&accept_pegin_txid)
        );
    }
}
