use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::{
    AdvanceFundsRegistered, AdvanceFundsRequest, BitVmxProtocolId, BtcTxSPVProof, CommsAddress,
    FundsAdvanceSPV, IncomingBitVMXApiMessages, OPERATOR_TAKE_TX, VariableTypes,
    accept_pegin_protocol_id, advance_funds_protocol_id,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{Hash256, TxHash};
use serde_json::json;
use tracing::{debug, info, trace};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::{
    GetCommitteeInput, RegisterAdvanceFundsInput, RequestPeginInput,
};
use union_contracts::bindings::pegout_manager::PegoutManager::PegoutRegistered;
use uuid::Uuid;

use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::flows::common::{FlowId, Signaling};
use crate::flows::operator_take::types::OperatorTakeTriggerData;
use crate::flows::pegout::pegout_flow::flow_id_from_pegout_requested_tx_hash;

pub(crate) const PROGRAM_TYPE_ADVANCE_FUNDS: &str = "advance_funds";
pub(crate) const ADVANCE_FUNDS_REQUEST_VAR_NAME: &str = "advance_funds_request";

/// Derive the operator-take flow id from the `OperatorTakeTriggered` RSK tx hash.
///
/// Deterministic and canonical: every operator's coordinator computes the
/// same `FlowId` for the same trigger event, so flow identity is consistent
/// across the network without a separate handshake.
#[must_use]
pub(crate) fn flow_id_from_operator_take_triggered_tx_hash(tx_hash: TxHash) -> FlowId {
    FlowId::from_tx("operator_take_flow", tx_hash.value().as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Steps {
    /// Entry point; no fast-forward.
    #[default]
    WaitRskOperatorTakeTriggered,
    /// Selected-only
    GetBitVmxCommInfo,
    /// Selected-only
    RequestBitVmxOperatorTakeTransactionInfo,
    /// Selected-only
    SetupBitVmxAdvanceFundsProtocol,
    /// Selected-only
    WaitBitVmxAdvanceFundsSpv,
    /// Selected submits `registerAdvanceFunds`; all wait for `AdvanceFundsRegistered`. Non-selected may fast-forward in from path start.
    RegisterOrWaitRskAdvanceFunds,
    /// All push `AdvanceFundsRegistered` `SetVar`
    /// Then wait for kickoff SPV. Checkpoint — gates later fast-forwards.
    SetVarBitVmxAdvanceFundsRegistered,
    /// Selected submits `registerReimbursementKickoff`. All wait for `ReimbursementKickoffRegistered`. Non-selected may fast-forward in from checkpoint.
    RegisterOrWaitRskReimbursementKickoff,
    /// Wait for operator-take SPV. Non-selected may fast-forward in from checkpoint.
    WaitBitVmxOperatorTakeSpv,
    /// Selected submits `registerOperatorTake`; all wait for `PegoutRegistered`. Non-selected may fast-forward in from checkpoint.
    RegisterOrWaitRskOperatorTake,
    Done,
    Failed,
}

impl Steps {
    /// Linear position along the selected operator's path. Non-selected
    /// operators traverse a strict skip-subset of the same ordering. `Failed`
    /// returns `u32::MAX`. If a future change makes a non-selected diverging off
    /// this path, this ordering breaks and the flow should be split per role.
    fn pos(self) -> u32 {
        match self {
            Steps::WaitRskOperatorTakeTriggered => 0,
            Steps::GetBitVmxCommInfo => 1,
            Steps::RequestBitVmxOperatorTakeTransactionInfo => 2,
            Steps::SetupBitVmxAdvanceFundsProtocol => 3,
            Steps::WaitBitVmxAdvanceFundsSpv => 4,
            Steps::RegisterOrWaitRskAdvanceFunds => 5,
            Steps::SetVarBitVmxAdvanceFundsRegistered => 6,
            Steps::RegisterOrWaitRskReimbursementKickoff => 7,
            Steps::WaitBitVmxOperatorTakeSpv => 8,
            Steps::RegisterOrWaitRskOperatorTake => 9,
            Steps::Done => 10,
            Steps::Failed => u32::MAX,
        }
    }

    fn is_past(self, other: Steps) -> bool {
        self.pos() > other.pos()
    }

    /// Selected: true if `self == target`
    /// Non-selected: `self ∈ [floor, target]`, where floor:
    ///     - is the `SetVar` checkpoint when `target` is at/past it
    ///     - otherwise the first step
    fn is_valid_transition(self, target: Steps, is_selected: bool) -> bool {
        if is_selected {
            return self == target;
        }

        let first = Steps::WaitRskOperatorTakeTriggered.pos();
        let checkpoint = Steps::SetVarBitVmxAdvanceFundsRegistered.pos();

        let max = target.pos();
        let current = self.pos();
        let min = if max < checkpoint { first } else { checkpoint };

        current >= min && current <= max
    }

    fn is_retriable_step(self) -> bool {
        matches!(
            self,
            Steps::RegisterOrWaitRskAdvanceFunds
                | Steps::RegisterOrWaitRskReimbursementKickoff
                | Steps::RegisterOrWaitRskOperatorTake
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) enum StepData {
    OperatorTakeTriggered,
    CommInfo(CommsAddress),
    SetupCompleted,
    OperatorTakeTransactionInfo(Txid),
    AdvanceFundsSPV(FundsAdvanceSPV),
    AdvanceFundsConfirmed(AdvanceFundsRegistered),
    ReimbursementKickoffSPV(BtcTxSPVProof),
    ReimbursementKickoffConfirmed,
    OperatorTakeSPV(BtcTxSPVProof),
    PegoutRegistered(PegoutRegistered),
    /// Generic retry kick from the processor's `RetryTracker`. The flow's
    /// current step decides what to retry — the processor doesn't need to know.
    Retry,
}

/// Result of `complete_step` (and `start_step`).
///
/// - `Done`: event was handled and state advanced.
/// - `NoOp`: event was handled with no state change. Used for explicitly
///   whitelisted `at-least-once` events (currently the three `BitVMX` SPV
///   notifications) that arrive after the flow has advanced past their
///   handler step. Any other late event still surfaces as `Err`.
/// - `Retry { reason }`: step's side effect can't proceed for a retryable
///   reason; the flow stays parked. The reason originates where the cause
///   is detected (inside the flow) and is consumed by the processor when
///   scheduling the retry tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepOutcome {
    Done,
    NoOp,
    Retry { reason: String },
}

#[derive(Debug, Clone)]
struct FlowContext {
    flow_id: FlowId,
    /// Cached `BitVMX` protocol id for this advance-funds protocol instance.
    /// Derived from `(committee_id, slot_index)` via `advance_funds_protocol_id`
    /// at flow construction and reused on every `BitVMX`-bound message. Kept
    /// distinct from `flow_id` because the `BitVMX` side keys protocols by
    /// committee+slot, while the coordinator's `FlowId` is derived from the
    /// canonical trigger tx hash.
    bitvmx_protocol_id: BitVmxProtocolId,
    step: Steps,
    trigger_data: OperatorTakeTriggerData,
    my_p2p_address: Option<CommsAddress>,
    accept_pegin_txid: Option<alloy_primitives::FixedBytes<32>>,
    my_committee_index: Option<usize>,
    operator_take_txid: Option<Txid>,
    advance_funds_spv: Option<FundsAdvanceSPV>,
    advance_funds_registered: Option<AdvanceFundsRegistered>,
    reimbursement_kickoff_spv: Option<BtcTxSPVProof>,
    operator_take_spv: Option<BtcTxSPVProof>,
    accept_pegin_pid: Option<Uuid>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) struct AdvanceFundsFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    signaling: Rc<Signaling>,
    state: FlowContext,
}

impl<CG, BC> AdvanceFundsFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub(crate) fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        signaling: Rc<Signaling>,
        flow_id: FlowId,
        trigger_data: OperatorTakeTriggerData,
    ) -> Self {
        let committee_uuid = Uuid::from_u128(*trigger_data.committee_id);
        let bitvmx_protocol_id = advance_funds_protocol_id(committee_uuid, trigger_data.slot_index);
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            native_bridge_verifier,
            signaling,
            state: FlowContext {
                flow_id,
                bitvmx_protocol_id,
                step: Steps::WaitRskOperatorTakeTriggered,
                trigger_data,
                my_p2p_address: None,
                accept_pegin_txid: None,
                my_committee_index: None,
                operator_take_txid: None,
                advance_funds_spv: None,
                advance_funds_registered: None,
                reimbursement_kickoff_spv: None,
                operator_take_spv: None,
                accept_pegin_pid: None,
                created_at: Some(chrono::Utc::now()),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        contracts: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        flow_id: FlowId,
        trigger_data: OperatorTakeTriggerData,
        step: Steps,
    ) -> Self {
        let committee_uuid = Uuid::from_u128(*trigger_data.committee_id);
        let bitvmx_protocol_id = advance_funds_protocol_id(committee_uuid, trigger_data.slot_index);
        Self {
            contracts,
            rt_sync: RuntimeSync::new().expect("Failed to create runtime sync for test flow"),
            bitvmx_broker,
            native_bridge_verifier: NativeBridgeVerifier::Dummy,
            signaling: Rc::new(Signaling::new("/tmp", "disabled")),
            state: FlowContext {
                flow_id,
                bitvmx_protocol_id,
                step,
                trigger_data,
                my_p2p_address: None,
                accept_pegin_txid: None,
                my_committee_index: None,
                operator_take_txid: None,
                advance_funds_spv: None,
                advance_funds_registered: None,
                reimbursement_kickoff_spv: None,
                operator_take_spv: None,
                accept_pegin_pid: None,
                created_at: None,
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn start_step(&mut self, next_step: Steps) -> Result<StepOutcome> {
        let previous_step = self.state.step;
        self.state.step = next_step;

        debug!(
            "AdvanceFundsFlow {}: {} -> {}",
            self.state.flow_id,
            format_step(previous_step),
            format_step(next_step)
        );

        match next_step {
            Steps::WaitRskOperatorTakeTriggered => unreachable!(
                "OperatorTakeTriggered is the initial step and should not be started explicitly"
            ),
            Steps::GetBitVmxCommInfo => self.enter_request_comm_info()?,
            Steps::RequestBitVmxOperatorTakeTransactionInfo => {
                self.enter_request_transaction_info()?;
            }
            Steps::SetupBitVmxAdvanceFundsProtocol => self.enter_setup_protocol()?,
            Steps::WaitBitVmxAdvanceFundsSpv => info!(
                "Waiting for advance-funds SPV proof for flow_id: {}, operator_take_txid: {}",
                self.state.flow_id,
                self.state
                    .operator_take_txid
                    .map_or_else(|| "n/a".to_string(), |txid| txid.to_string()),
            ),
            Steps::RegisterOrWaitRskAdvanceFunds => {
                if self.was_selected_operator() {
                    return self.enter_register_advance_funds();
                }
                info!(
                    "Waiting for advance-funds confirmations on Rootstock for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::SetVarBitVmxAdvanceFundsRegistered => self.enter_notify_registered()?,
            Steps::RegisterOrWaitRskReimbursementKickoff => {
                if self.was_selected_operator() {
                    return self.enter_register_reimbursement_kickoff();
                }
                info!(
                    "Waiting for reimbursement-kickoff confirmations on Rootstock for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::WaitBitVmxOperatorTakeSpv => {
                info!("Waiting for operator take SPV proof for flow_id: {}", self.state.flow_id);
            }
            Steps::RegisterOrWaitRskOperatorTake => {
                if self.was_selected_operator() {
                    return self.enter_register_operator_take();
                }
                info!(
                    "Waiting for operator-take confirmations on Rootstock for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::Done => self.enter_write_completion_marker()?,
            Steps::Failed => info!("AdvanceFundsFlow {}: Failed", self.state.flow_id),
        }

        Ok(StepOutcome::Done)
    }

    /// Deliver an event (`StepData`) to the flow. Dispatches to the
    /// per-event handler, which validates the current step and returns the
    /// next step; then runs that step's entry side effects via `start_step`.
    /// The dispatch match is exhaustive over `StepData` — adding a new
    /// variant requires wiring up a handler here.
    pub(crate) fn complete_step(&mut self, data: StepData) -> Result<StepOutcome> {
        let current_step = self.state.step;

        debug!(
            "AdvanceFundsFlow {}: Completing step {} with data: {:?}",
            self.state.flow_id,
            format_step(current_step),
            data
        );

        let next_step = match data {
            StepData::OperatorTakeTriggered => self.on_operator_take_triggered(current_step)?,
            StepData::CommInfo(comm_info) => self.on_comm_info(current_step, comm_info)?,
            StepData::SetupCompleted => self.on_setup_completed(current_step)?,
            StepData::OperatorTakeTransactionInfo(txid) => {
                self.on_operator_take_transaction_info(current_step, txid)?
            }
            StepData::AdvanceFundsSPV(spv_data) => {
                self.on_advance_funds_spv(current_step, spv_data)?
            }
            StepData::AdvanceFundsConfirmed(data) => {
                self.on_advance_funds_confirmed(current_step, data)?
            }
            StepData::ReimbursementKickoffSPV(spv_proof) => {
                self.on_reimbursement_kickoff_spv(current_step, spv_proof)?
            }
            StepData::ReimbursementKickoffConfirmed => {
                self.on_reimbursement_kickoff_confirmed(current_step)?
            }
            StepData::OperatorTakeSPV(spv_proof) => {
                self.on_operator_take_spv(current_step, spv_proof)?
            }
            StepData::PegoutRegistered(pegout_registered) => {
                self.on_pegout_registered(current_step, &pegout_registered)?
            }
            StepData::Retry => self.on_retry(current_step),
        };

        match next_step {
            Some(next) => self.start_step(next),
            None => Ok(StepOutcome::NoOp),
        }
    }

    fn on_operator_take_triggered(&mut self, current_step: Steps) -> Result<Option<Steps>> {
        if current_step != Steps::WaitRskOperatorTakeTriggered {
            bail!("Invalid state transition from {current_step:?} with OperatorTakeTriggered");
        }
        if self.was_selected_operator() {
            Ok(Some(Steps::GetBitVmxCommInfo))
        } else {
            Ok(Some(Steps::RegisterOrWaitRskAdvanceFunds))
        }
    }

    fn on_comm_info(
        &mut self,
        current_step: Steps,
        comm_info: CommsAddress,
    ) -> Result<Option<Steps>> {
        if current_step != Steps::GetBitVmxCommInfo {
            bail!("Invalid state transition from {current_step:?} with CommInfo");
        }
        self.state.my_p2p_address = Some(comm_info);
        Ok(Some(Steps::RequestBitVmxOperatorTakeTransactionInfo))
    }

    fn on_operator_take_transaction_info(
        &mut self,
        current_step: Steps,
        txid: Txid,
    ) -> Result<Option<Steps>> {
        if current_step != Steps::RequestBitVmxOperatorTakeTransactionInfo {
            bail!(
                "Invalid state transition from {current_step:?} with OperatorTakeTransactionInfo"
            );
        }
        self.state.operator_take_txid = Some(txid);
        Ok(Some(Steps::SetupBitVmxAdvanceFundsProtocol))
    }

    // `&mut self` kept for symmetry with the other `on_*` handlers; this one
    // happens not to read or mutate state.
    #[allow(clippy::unused_self)]
    fn on_setup_completed(&mut self, current_step: Steps) -> Result<Option<Steps>> {
        if current_step != Steps::SetupBitVmxAdvanceFundsProtocol {
            bail!("Invalid state transition from {current_step:?} with SetupCompleted");
        }
        Ok(Some(Steps::WaitBitVmxAdvanceFundsSpv))
    }

    fn on_advance_funds_spv(
        &mut self,
        current_step: Steps,
        spv_data: FundsAdvanceSPV,
    ) -> Result<Option<Steps>> {
        // BitVMX re-emits SPVs on every block-confirmation update (at-least-once); absorb late duplicates here. One-shot events don't need this.
        if current_step.is_past(Steps::WaitBitVmxAdvanceFundsSpv) {
            debug!(
                "Dropping stale AdvanceFundsSPV at {current_step:?} for flow_id {}",
                self.state.flow_id
            );
            return Ok(None);
        }
        if current_step != Steps::WaitBitVmxAdvanceFundsSpv {
            bail!("Invalid state transition from {current_step:?} with AdvanceFundsSPV");
        }
        info!(
            "Advance funds SPV received for flow_id {} - txid: {}, committee: {}, slot: {}",
            self.state.flow_id, spv_data.txid, spv_data.committee_id, spv_data.slot_index
        );
        self.state.advance_funds_spv = Some(spv_data);
        Ok(Some(Steps::RegisterOrWaitRskAdvanceFunds))
    }

    fn on_advance_funds_confirmed(
        &mut self,
        current_step: Steps,
        data: AdvanceFundsRegistered,
    ) -> Result<Option<Steps>> {
        if !current_step
            .is_valid_transition(Steps::RegisterOrWaitRskAdvanceFunds, self.was_selected_operator())
        {
            bail!("Invalid state transition from {current_step:?} with AdvanceFundsConfirmed");
        }
        self.state.advance_funds_registered = Some(data);
        Ok(Some(Steps::SetVarBitVmxAdvanceFundsRegistered))
    }

    fn on_reimbursement_kickoff_spv(
        &mut self,
        current_step: Steps,
        spv_proof: BtcTxSPVProof,
    ) -> Result<Option<Steps>> {
        // BitVMX re-emits SPVs on every block-confirmation update (at-least-once); absorb late duplicates here. One-shot events don't need this.
        if current_step.is_past(Steps::SetVarBitVmxAdvanceFundsRegistered) {
            debug!(
                "Dropping stale ReimbursementKickoffSPV at {current_step:?} for flow_id {}",
                self.state.flow_id
            );
            return Ok(None);
        }
        if !current_step.is_valid_transition(
            Steps::SetVarBitVmxAdvanceFundsRegistered,
            self.was_selected_operator(),
        ) {
            bail!("Invalid state transition from {current_step:?} with ReimbursementKickoffSPV");
        }
        info!("Reimbursement kickoff SPV received for flow_id {}", self.state.flow_id);
        if self.was_selected_operator() {
            self.state.reimbursement_kickoff_spv = Some(spv_proof);
        }
        Ok(Some(Steps::RegisterOrWaitRskReimbursementKickoff))
    }

    fn on_reimbursement_kickoff_confirmed(&mut self, current_step: Steps) -> Result<Option<Steps>> {
        if !current_step.is_valid_transition(
            Steps::RegisterOrWaitRskReimbursementKickoff,
            self.was_selected_operator(),
        ) {
            bail!(
                "Invalid state transition from {current_step:?} with ReimbursementKickoffConfirmed"
            );
        }
        // Both paths converge on the operator-take SPV wait step: each
        // operator's accept_pegin observes the BTC OPERATOR_TAKE_TX and emits
        // the SPV.
        Ok(Some(Steps::WaitBitVmxOperatorTakeSpv))
    }

    fn on_operator_take_spv(
        &mut self,
        current_step: Steps,
        spv_proof: BtcTxSPVProof,
    ) -> Result<Option<Steps>> {
        // BitVMX re-emits SPVs on every block-confirmation update (at-least-once); absorb late duplicates here. One-shot events don't need this.
        if current_step.is_past(Steps::WaitBitVmxOperatorTakeSpv) {
            debug!(
                "Dropping stale OperatorTakeSPV at {current_step:?} for flow_id {}",
                self.state.flow_id
            );
            return Ok(None);
        }
        if !current_step
            .is_valid_transition(Steps::WaitBitVmxOperatorTakeSpv, self.was_selected_operator())
        {
            bail!("Invalid state transition from {current_step:?} with OperatorTakeSPV");
        }
        info!("Operator take SPV received for flow_id {}", self.state.flow_id);
        if self.was_selected_operator() {
            self.state.operator_take_spv = Some(spv_proof);
        }
        Ok(Some(Steps::RegisterOrWaitRskOperatorTake))
    }

    fn on_pegout_registered(
        &mut self,
        current_step: Steps,
        pegout_registered: &PegoutRegistered,
    ) -> Result<Option<Steps>> {
        if !current_step
            .is_valid_transition(Steps::RegisterOrWaitRskOperatorTake, self.was_selected_operator())
        {
            bail!("Invalid state transition from {current_step:?} with PegoutRegistered");
        }
        debug!("Operator take registered for flow {}: {:?}", self.state.flow_id, pegout_registered);
        Ok(Some(Steps::Done))
    }

    fn on_retry(&mut self, current_step: Steps) -> Option<Steps> {
        if !current_step.is_retriable_step() {
            debug!(
                "Stale retry for flow_id {} at non-retriable step {current_step:?}; success event already advanced the flow",
                self.state.flow_id
            );
            return None;
        }
        info!("Retrying registration at step {current_step:?} for flow_id: {}", self.state.flow_id);
        Some(current_step)
    }

    fn enter_setup_protocol(&mut self) -> Result<()> {
        if !self.was_selected_operator() {
            bail!("Only the selected operator can set up the advance funds protocol");
        }
        info!("Setting up advance funds protocol for flow_id: {}", self.state.flow_id);

        let my_p2p_address = self
            .state
            .my_p2p_address
            .clone()
            .ok_or_else(|| anyhow!("P2P address not available for advance funds setup"))?;

        let participants = vec![my_p2p_address.clone()];

        let operator_pubkey = self.state.trigger_data.operator_take_pubkey;
        let request_payload = self.build_advance_funds_request(operator_pubkey)?;
        let bitvmx_pid = self.bitvmx_protocol_id().value();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            bitvmx_pid,
            ADVANCE_FUNDS_REQUEST_VAR_NAME.to_string(),
            VariableTypes::String(request_payload),
        ))?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::Setup(
            bitvmx_pid,
            PROGRAM_TYPE_ADVANCE_FUNDS.to_string(),
            participants,
            0,
        ))
    }

    fn enter_register_operator_take(&self) -> Result<StepOutcome> {
        info!("Registering operator take for flow_id: {}", self.state.flow_id);
        let spv_proof = self
            .state
            .operator_take_spv
            .as_ref()
            .ok_or_else(|| anyhow!("Operator take SPV not available"))?;
        let input: RequestPeginInput = spv_proof.clone().into();
        let output = match invoke_contract_safe(
            &self.rt_sync,
            "registerOperatorTake",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.register_operator_take(input).await },
        ) {
            Ok(output) => output,
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(detail)) => {
                return Ok(StepOutcome::Retry {
                    reason: format!(
                        "native bridge missing confirmations for operator-take registration: {detail}"
                    ),
                });
            }
            Err(other) => {
                return Err(other)
                    .context("Failed to register operator take with provided SPV proof");
            }
        };

        info!(
            "Operator take registration sent for flow_id {} with tx hash {}, waiting Rootstock confirmations",
            self.state.flow_id, output.transaction_hash
        );

        Ok(StepOutcome::Done)
    }

    fn resolve_accept_pegin_txid(&mut self) -> Result<alloy_primitives::FixedBytes<32>> {
        if let Some(cached) = self.state.accept_pegin_txid {
            return Ok(cached);
        }

        let pegout_txid = self.state.trigger_data.pegout_txid.into();
        let input = transaction_dispatcher::types::GetAcceptPeginTxidInput { pegout_txid };
        let output = self
            .rt_sync
            .run(async { self.contracts.get_accept_pegin_txid(input).await })
            .context("Failed to get accept pegin txid for pegout")?;

        self.state.accept_pegin_txid = Some(output.accept_pegin_txid);
        Ok(output.accept_pegin_txid)
    }

    fn enter_register_advance_funds(&mut self) -> Result<StepOutcome> {
        info!("Registering advance funds for flow_id: {}", self.state.flow_id);
        let spv_proof = self
            .state
            .advance_funds_spv
            .as_ref()
            .ok_or_else(|| anyhow!("Advance funds SPV data not available"))?
            .spv_proof
            .clone();
        let input: RequestPeginInput = spv_proof.clone().into();
        let accept_pegin_txid = self.resolve_accept_pegin_txid()?;

        let register_input =
            RegisterAdvanceFundsInput { accept_pegin_txid, advance_funds_spv_proof: input };

        // Clone Rc fields to avoid conflicting borrows in the async closure
        // while `&mut self` is still in scope from resolve_accept_pegin_txid.
        let contracts = Rc::clone(&self.contracts);
        let output = match invoke_contract_safe(
            &self.rt_sync,
            "registerAdvanceFunds",
            &spv_proof,
            &self.native_bridge_verifier,
            || async move { contracts.register_advance_funds(register_input).await },
        ) {
            Ok(output) => output,
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(detail)) => {
                return Ok(StepOutcome::Retry {
                    reason: format!(
                        "native bridge missing confirmations for advance-funds registration: {detail}"
                    ),
                });
            }
            Err(other) => return Err(other).context("Failed to register advance funds SPV proof"),
        };

        info!(
            "Advance funds registration sent for flow_id {} with tx hash {}, waiting Rootstock confirmations",
            self.state.flow_id, output.transaction_hash
        );

        Ok(StepOutcome::Done)
    }

    fn enter_register_reimbursement_kickoff(&mut self) -> Result<StepOutcome> {
        info!("Registering reimbursement kickoff for flow_id: {}", self.state.flow_id);
        let spv_proof = self
            .state
            .reimbursement_kickoff_spv
            .as_ref()
            .ok_or_else(|| anyhow!("Reimbursement kickoff SPV not available"))?
            .clone();
        let input: RequestPeginInput = spv_proof.clone().into();
        let accept_pegin_txid = self.resolve_accept_pegin_txid()?;

        let register_input = transaction_dispatcher::types::RegisterReimbursementKickoffInput {
            accept_pegin_txid,
            kickoff_spv_proof: input,
        };

        // Clone Rc fields to avoid conflicting borrows in the async closure
        // while `&mut self` is still in scope from resolve_accept_pegin_txid.
        let contracts = Rc::clone(&self.contracts);
        let output = match invoke_contract_safe(
            &self.rt_sync,
            "registerReimbursementKickoff",
            &spv_proof,
            &self.native_bridge_verifier,
            || async move { contracts.register_reimbursement_kickoff(register_input).await },
        ) {
            Ok(output) => output,
            Err(DomainErrors::MissingConfirmationsOnNativeBridge(detail)) => {
                return Ok(StepOutcome::Retry {
                    reason: format!(
                        "native bridge missing confirmations for reimbursement-kickoff registration: {detail}"
                    ),
                });
            }
            Err(other) => {
                return Err(other).context("Failed to register reimbursement kickoff SPV proof");
            }
        };

        info!(
            "Reimbursement-kickoff registration sent for flow_id {} with tx hash {}, waiting Rootstock confirmations",
            self.state.flow_id, output.transaction_hash
        );

        Ok(StepOutcome::Done)
    }

    fn enter_request_comm_info(&self) -> Result<()> {
        info!("Requesting BitVMX comm info for advance funds flow_id: {}", self.state.flow_id);
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id))
    }

    fn enter_request_transaction_info(&mut self) -> Result<()> {
        info!(
            "Requesting operator take transaction info from BitVMX for flow_id: {}",
            self.state.flow_id
        );
        let tx_name = self.operator_take_transaction_name()?;
        let accept_pegin_pid =
            accept_pegin_protocol_id(self.committee_id_uuid(), self.state.trigger_data.slot_index)
                .value();
        self.state.accept_pegin_pid = Some(accept_pegin_pid);
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransactionInfoByName(
            accept_pegin_pid,
            tx_name,
        ))
    }

    fn operator_take_transaction_name(&mut self) -> Result<String> {
        let member_index = self.my_committee_index()?;
        Ok(format!("{OPERATOR_TAKE_TX}_{member_index}"))
    }

    fn my_committee_index(&mut self) -> Result<usize> {
        if let Some(index) = self.state.my_committee_index {
            return Ok(index);
        }
        let committee_id = self.state.trigger_data.committee_id.clone();
        let committee_response = self
            .rt_sync
            .run(async { self.contracts.get_committee(GetCommitteeInput { committee_id }).await })
            .context("Failed to fetch committee for take operator index lookup")?;
        let my_addr: alloy_primitives::Address = self.contracts.my_address().into();
        let index = committee_response
            .committee
            .members
            .iter()
            .position(|member| member.memberAddress == my_addr)
            .context("Address not found in committee members")?;
        self.state.my_committee_index = Some(index);
        Ok(index)
    }

    fn committee_id_uuid(&self) -> Uuid {
        Uuid::from_u128(*self.state.trigger_data.committee_id)
    }

    fn build_advance_funds_request(&self, operator_pubkey: PublicKey) -> Result<String> {
        let pegout_id = self.state.trigger_data.pegout_id.value().as_bytes().to_vec();

        let payload: AdvanceFundsRequest = AdvanceFundsRequest {
            committee_id: Uuid::from_u128(*self.state.trigger_data.committee_id),
            slot_index: self.state.trigger_data.slot_index,
            pegout_id,
            fee: 335,
            user_pubkey: self.state.trigger_data.user_pubkey,
            my_take_pubkey: operator_pubkey,
        };

        serde_json::to_string(&payload)
            .context("Failed to encode advance funds request payload to JSON")
    }

    fn enter_notify_registered(&self) -> Result<()> {
        info!("Notifying BitVMX of advance funds registered for flow_id: {}", self.state.flow_id);

        let data =
            self.state.advance_funds_registered.as_ref().ok_or_else(|| {
                anyhow!("AdvanceFundsRegistered data not available for notification")
            })?;

        let key = AdvanceFundsRegistered::name(data.slot_index);
        let json = serde_json::to_string(data)
            .context("Failed to serialize AdvanceFundsRegistered for BitVMX")?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            data.committee_id,
            key,
            VariableTypes::String(json),
        ))?;

        info!("Waiting for reimbursement kickoff SPV for flow_id: {}", self.state.flow_id);
        Ok(())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("AdvanceFundsFlow - sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    fn was_selected_operator(&self) -> bool {
        self.contracts.my_address() == self.state.trigger_data.take_operator_address
    }

    /// True when this flow is currently waiting for a `CommInfo` reply from
    /// `BitVMX`. Used by the processor as a routing predicate since `BitVMX`'s
    /// `CommInfo` message doesn't carry a `program_id` (only a `req_id` we don't
    /// thread back through state today).
    pub(crate) fn is_waiting_comm_info(&self) -> bool {
        self.state.step == Steps::GetBitVmxCommInfo
    }

    /// True when this flow is at the operator-take txid request step and its
    /// captured `accept_pegin_pid` matches `program_id`. The step check guards
    /// against late replies arriving after the flow advanced.
    pub(crate) fn matches_accept_pegin_pid(&self, program_id: &Uuid) -> bool {
        self.state.step == Steps::RequestBitVmxOperatorTakeTransactionInfo
            && self.state.accept_pegin_pid == Some(*program_id)
    }

    fn enter_write_completion_marker(&self) -> Result<()> {
        let payload = json!({
            "ancestor_pegout_id": self.ancestor_pegout_id(),
            "request_pegout_tx_hash": self.state.trigger_data.request_pegout_tx_hash,
            "pegout_txid": self.state.trigger_data.pegout_txid.to_string(),
            "pegout_id": self.state.trigger_data.pegout_id.to_string(),
            "committee_id": self.state.trigger_data.committee_id.to_string(),
            "slot_id": self.state.trigger_data.slot_id,
            "slot_index": self.state.trigger_data.slot_index,
            "selected_operator_address": self.state.trigger_data.take_operator_address.to_string(),
            "was_selected_operator": self.was_selected_operator(),
            "accept_pegin_txid": self.state.accept_pegin_txid.map(|txid| format!("{txid:#066x}")),
            "advance_funds_txid": self.state.advance_funds_registered.as_ref().map(|event| event.txid.to_string()),
        });

        // `signal_done` takes a `Uuid` for marker-file naming; render our
        // canonical `FlowId` as Uuid.
        self.signaling.signal_done("advance-funds", self.state.flow_id.value(), &payload)?;
        info!("AdvanceFundsFlow {}: Done", self.state.flow_id);
        Ok(())
    }

    pub(crate) fn flow_id(&self) -> FlowId {
        self.state.flow_id
    }

    pub(crate) fn slot_index(&self) -> usize {
        self.state.trigger_data.slot_index
    }

    pub(crate) fn ancestor_pegout_id(&self) -> String {
        self.state
            .trigger_data
            .request_pegout_tx_hash
            .as_deref()
            .and_then(|tx_hash| TxHash::try_from(tx_hash).ok())
            .map(flow_id_from_pegout_requested_tx_hash)
            .map_or_else(|| self.state.flow_id.to_string(), |flow_id| flow_id.to_string())
    }

    /// BitVMX-side protocol id for this flow's advance-funds protocol
    /// instance. Used as the `program_id` on `Setup` / `SetVar` etc.
    pub(crate) fn bitvmx_protocol_id(&self) -> BitVmxProtocolId {
        self.state.bitvmx_protocol_id
    }

    #[cfg(test)]
    pub(crate) fn current_step(&self) -> Steps {
        self.state.step
    }

    #[cfg(test)]
    pub(crate) fn has_advance_funds_registered(&self) -> bool {
        self.state.advance_funds_registered.is_some()
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self.state.step, Steps::Done | Steps::Failed)
    }

    /// Snapshot used by `Coordinator::log_active_flows` for periodic
    /// observability of in-flight flows.
    pub(crate) fn get_flow_details(&self) -> crate::event_processor::FlowDetails {
        crate::event_processor::FlowDetails {
            kind: crate::types::FlowKind::AdvanceFunds,
            id: self.flow_id().to_string(),
            step: format!("{:?}", self.state.step),
            created_at: self.state.created_at,
        }
    }

    pub(crate) fn mark_failed(&mut self, reason: &str) -> Result<()> {
        info!("Marking advance funds flow {} as failed: {reason}", self.state.flow_id);
        self.start_step(Steps::Failed).map(|_| ())
    }

    /// True when this flow's trigger matches the given `pegout_id`.
    pub(crate) fn matches_pegout(&self, pegout_id: Hash256) -> bool {
        self.state.trigger_data.pegout_id == pegout_id
    }

    /// True when this flow's trigger matches the given `(committee_id, slot_id)`.
    pub(crate) fn matches_committee_slot(&self, committee_id: u128, slot_id: u64) -> bool {
        *self.state.trigger_data.committee_id == committee_id
            && self.state.trigger_data.slot_id == slot_id
    }
}

fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::WaitRskOperatorTakeTriggered => "WaitRskOperatorTakeTriggered",
        Steps::GetBitVmxCommInfo => "GetBitVmxCommInfo",
        Steps::RequestBitVmxOperatorTakeTransactionInfo => {
            "RequestBitVmxOperatorTakeTransactionInfo"
        }
        Steps::SetupBitVmxAdvanceFundsProtocol => "SetupBitVmxAdvanceFundsProtocol",
        Steps::WaitBitVmxAdvanceFundsSpv => "WaitBitVmxAdvanceFundsSpv",
        Steps::RegisterOrWaitRskAdvanceFunds => "RegisterOrWaitRskAdvanceFunds",
        Steps::SetVarBitVmxAdvanceFundsRegistered => "SetVarBitVmxAdvanceFundsRegistered",
        Steps::RegisterOrWaitRskReimbursementKickoff => "RegisterOrWaitRskReimbursementKickoff",
        Steps::WaitBitVmxOperatorTakeSpv => "WaitBitVmxOperatorTakeSpv",
        Steps::RegisterOrWaitRskOperatorTake => "RegisterOrWaitRskOperatorTake",
        Steps::Done => "Done",
        Steps::Failed => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::str::FromStr;

    use alloy_primitives::FixedBytes;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{PublicKey, Transaction};
    use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::types::{Address, CommitteeId, Hash256};
    use primitive_types::{H160, H256};
    use union_contracts::bindings::pegout_manager::PegoutManager::StreamPosition;
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

    fn test_pegout_registered() -> PegoutRegistered {
        PegoutRegistered {
            blockHash: FixedBytes::from([1u8; 32]),
            txid: FixedBytes::from([2u8; 32]),
            acceptPeginTxid: FixedBytes::from([3u8; 32]),
            committeeId: 1,
            streamInfo: StreamPosition { streamId: 0, packetNumber: 0, slotId: 0, pegStatus: 0 },
        }
    }

    fn test_spv_proof_value() -> BtcTxSPVProof {
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
    fn non_selected_operator_skips_get_comm_info_and_waits_for_registration() {
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);

        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

        let broker = MockBitVmxBroker::new();

        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(broker),
            flow_id,
            trigger_data,
            Steps::WaitRskOperatorTakeTriggered,
        );

        flow.complete_step(StepData::OperatorTakeTriggered)
            .expect("non-selected operator should skip to waiting for on-chain registration");

        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskAdvanceFunds);
    }

    #[test]
    fn operator_take_registered_advances_selected_only_from_wait_step() {
        // Selected operators converge with non-selected at the shared wait step
        // after submitting registerOperatorTake, so the strict policy requires
        // them to be at WaitForRskPegoutRegisteredAllOps.
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);

        let mut contracts = MockRskContractsGatewayApi::new();
        // Match take_operator_address so was_selected_operator() returns true.
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(33)));

        let broker = MockBitVmxBroker::new();
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(broker),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskOperatorTake,
        );

        flow.complete_step(StepData::PegoutRegistered(test_pegout_registered()))
            .expect("selected at WaitForRskPegoutRegisteredAllOps should advance to Done");

        assert_eq!(flow.current_step(), Steps::Done);
    }

    #[test]
    fn operator_take_registered_fast_forwards_non_selected_past_setvar_checkpoint() {
        // Non-selected operators that have already passed through SetVar (pos >= 2)
        // can fast-forward to Done on PegoutRegistered.
        let non_selected_steps = [
            Steps::SetVarBitVmxAdvanceFundsRegistered,
            Steps::RegisterOrWaitRskReimbursementKickoff,
            Steps::WaitBitVmxOperatorTakeSpv,
            Steps::RegisterOrWaitRskOperatorTake,
        ];

        for step in non_selected_steps {
            let committee_id = Uuid::new_v4();
            let flow_id = FlowId::from_random();
            let trigger_data = test_trigger_data(committee_id, 0);

            let mut contracts = MockRskContractsGatewayApi::new();
            // Differs from take_operator_address (33) so was_selected_operator() is false.
            contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

            let broker = MockBitVmxBroker::new();
            let mut flow = AdvanceFundsFlow::new_for_test(
                Rc::new(contracts),
                Rc::new(broker),
                flow_id,
                trigger_data,
                step,
            );

            flow.complete_step(StepData::PegoutRegistered(test_pegout_registered()))
                .expect("non-selected past SetVar should fast-forward to Done");

            assert_eq!(flow.current_step(), Steps::Done, "failed from {step:?}");
        }
    }

    #[test]
    fn operator_take_registered_is_blocked_for_non_selected_before_setvar() {
        // The SetVar push is a checkpoint: non-selected operators before SetVar
        // (pos 0 or 1) must not skip it via fast-forward. The event surfaces
        // as an error so the operator can investigate (likely a state-
        // divergence / restart-lag scenario tracked separately).
        let pre_setvar_steps =
            [Steps::WaitRskOperatorTakeTriggered, Steps::RegisterOrWaitRskAdvanceFunds];

        for step in pre_setvar_steps {
            let committee_id = Uuid::new_v4();
            let flow_id = FlowId::from_random();
            let trigger_data = test_trigger_data(committee_id, 0);

            let mut contracts = MockRskContractsGatewayApi::new();
            contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

            let broker = MockBitVmxBroker::new();
            let mut flow = AdvanceFundsFlow::new_for_test(
                Rc::new(contracts),
                Rc::new(broker),
                flow_id,
                trigger_data,
                step,
            );

            let result = flow.complete_step(StepData::PegoutRegistered(test_pegout_registered()));

            assert!(
                result.is_err(),
                "event before SetVar should surface as error, not silent no-op"
            );
            assert_eq!(flow.current_step(), step, "should stay at {step:?}, not jump past SetVar");
        }
    }

    #[test]
    fn reimbursement_kickoff_spv_is_blocked_for_non_selected_before_setvar() {
        // ReimbursementKickoffSPV gated by the SetVar checkpoint for non-selected.
        // Arriving before checkpoint (pos 0 or 1) must error rather than fast-forward.
        let pre_setvar_steps =
            [Steps::WaitRskOperatorTakeTriggered, Steps::RegisterOrWaitRskAdvanceFunds];

        for step in pre_setvar_steps {
            let committee_id = Uuid::new_v4();
            let flow_id = FlowId::from_random();
            let trigger_data = test_trigger_data(committee_id, 0);

            let mut contracts = MockRskContractsGatewayApi::new();
            contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

            let mut flow = AdvanceFundsFlow::new_for_test(
                Rc::new(contracts),
                Rc::new(MockBitVmxBroker::new()),
                flow_id,
                trigger_data,
                step,
            );

            let result =
                flow.complete_step(StepData::ReimbursementKickoffSPV(test_spv_proof_value()));

            assert!(
                result.is_err(),
                "kickoff SPV before SetVar should surface as error at {step:?}"
            );
            assert_eq!(flow.current_step(), step, "should stay at {step:?}, not jump past SetVar");
        }
    }

    #[test]
    fn operator_take_spv_is_blocked_for_non_selected_before_setvar() {
        // OperatorTakeSPV gated by the SetVar checkpoint for non-selected.
        // Arriving before checkpoint (pos 0 or 1) must error rather than fast-forward.
        let pre_setvar_steps =
            [Steps::WaitRskOperatorTakeTriggered, Steps::RegisterOrWaitRskAdvanceFunds];

        for step in pre_setvar_steps {
            let committee_id = Uuid::new_v4();
            let flow_id = FlowId::from_random();
            let trigger_data = test_trigger_data(committee_id, 0);

            let mut contracts = MockRskContractsGatewayApi::new();
            contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

            let mut flow = AdvanceFundsFlow::new_for_test(
                Rc::new(contracts),
                Rc::new(MockBitVmxBroker::new()),
                flow_id,
                trigger_data,
                step,
            );

            let result = flow.complete_step(StepData::OperatorTakeSPV(test_spv_proof_value()));

            assert!(
                result.is_err(),
                "operator-take SPV before SetVar should surface as error at {step:?}"
            );
            assert_eq!(flow.current_step(), step, "should stay at {step:?}, not jump past SetVar");
        }
    }

    #[test]
    fn operator_take_registered_at_non_fast_forwardable_step_errors() {
        // A selected operator at a step that doesn't match any arm for
        // `OperatorTakeRegistered`: the catch-all in process_step_data surfaces
        // as Err, the event loop sees it, and the operator gets a loud signal.
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);
        let mut contracts = MockRskContractsGatewayApi::new();
        // Match take_operator_address so was_selected_operator() returns true.
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(33)));
        let broker = MockBitVmxBroker::new();
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(broker),
            flow_id,
            trigger_data,
            Steps::WaitBitVmxAdvanceFundsSpv,
        );

        let result = flow.complete_step(StepData::PegoutRegistered(test_pegout_registered()));

        assert!(result.is_err(), "non-matching event should surface as error");
        assert_eq!(flow.current_step(), Steps::WaitBitVmxAdvanceFundsSpv);
    }

    fn test_funds_advance_spv() -> FundsAdvanceSPV {
        FundsAdvanceSPV {
            committee_id: Uuid::nil(),
            slot_index: 0,
            txid: test_spv_proof_value().tx.compute_txid(),
            pegout_id: vec![0u8; 32],
            spv_proof: test_spv_proof_value(),
        }
    }

    #[test]
    fn stale_advance_funds_spv_after_handler_is_noop() {
        // BitVMX re-emits SPVs on every block-confirmation update. After the
        // flow has advanced past WaitBitVmxAdvanceFundsSpv, the SPV is stale
        // and must drop silently as NoOp (no error, no state change).
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(33)));
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskAdvanceFunds, // strictly past WaitBitVmxAdvanceFundsSpv
        );

        let outcome = flow
            .complete_step(StepData::AdvanceFundsSPV(test_funds_advance_spv()))
            .expect("stale AdvanceFundsSPV should drop silently");

        assert_eq!(outcome, StepOutcome::NoOp);
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskAdvanceFunds);
    }

    #[test]
    fn stale_reimbursement_kickoff_spv_after_handler_is_noop() {
        // Same at-least-once semantics: stale ReimbursementKickoffSPV at a
        // post-handler step is NoOp.
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);
        let mut contracts = MockRskContractsGatewayApi::new();
        // Non-selected so we can place the flow past SetVar via fast-forward
        // without needing to run the selected-only register action.
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskReimbursementKickoff, // past SetVar
        );

        let outcome = flow
            .complete_step(StepData::ReimbursementKickoffSPV(test_spv_proof_value()))
            .expect("stale ReimbursementKickoffSPV should drop silently");

        assert_eq!(outcome, StepOutcome::NoOp);
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskReimbursementKickoff);
    }

    #[test]
    fn stale_operator_take_spv_after_handler_is_noop() {
        // Same at-least-once semantics: stale OperatorTakeSPV at a
        // post-handler step is NoOp.
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::RegisterOrWaitRskOperatorTake, // past WaitBitVmxOperatorTakeSpv
        );

        let outcome = flow
            .complete_step(StepData::OperatorTakeSPV(test_spv_proof_value()))
            .expect("stale OperatorTakeSPV should drop silently");

        assert_eq!(outcome, StepOutcome::NoOp);
        assert_eq!(flow.current_step(), Steps::RegisterOrWaitRskOperatorTake);
    }

    #[test]
    fn late_one_shot_event_still_surfaces_as_error() {
        // Negative case for the at-least-once whitelist: non-whitelisted events
        // arriving late must still surface as Err. The stale-NoOp policy is
        // explicit per-event, not a blanket "late = no-op".
        let committee_id = Uuid::new_v4();
        let flow_id = FlowId::from_random();
        let trigger_data = test_trigger_data(committee_id, 0);
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));
        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(MockBitVmxBroker::new()),
            flow_id,
            trigger_data,
            Steps::Done,
        );

        // ReimbursementKickoffConfirmed is a one-shot RSK event, not on the
        // at-least-once whitelist. Late arrival at Done must error.
        let result = flow.complete_step(StepData::ReimbursementKickoffConfirmed);

        assert!(result.is_err(), "late one-shot event should surface as error");
        assert_eq!(flow.current_step(), Steps::Done);
    }

    #[test]
    fn pos_total_ordering_is_strictly_monotonic_along_selected_path() {
        // The pos() table assumes non-selected operators traverse a strict
        // skip-subset of selected. Encoded as: every step in the selected
        // path has a strictly greater pos than its predecessor. Failed is
        // off the path and excluded.
        let selected_path = [
            Steps::WaitRskOperatorTakeTriggered,
            Steps::GetBitVmxCommInfo,
            Steps::RequestBitVmxOperatorTakeTransactionInfo,
            Steps::SetupBitVmxAdvanceFundsProtocol,
            Steps::WaitBitVmxAdvanceFundsSpv,
            Steps::RegisterOrWaitRskAdvanceFunds,
            Steps::SetVarBitVmxAdvanceFundsRegistered,
            Steps::RegisterOrWaitRskReimbursementKickoff,
            Steps::WaitBitVmxOperatorTakeSpv,
            Steps::RegisterOrWaitRskOperatorTake,
            Steps::Done,
        ];

        for window in selected_path.windows(2) {
            let (a, b) = (window[0], window[1]);
            assert!(
                a.pos() < b.pos(),
                "pos must be strictly monotonic along the selected path: {a:?}({}) < {b:?}({})",
                a.pos(),
                b.pos()
            );
        }
        assert_eq!(Steps::Failed.pos(), u32::MAX);
    }
}
