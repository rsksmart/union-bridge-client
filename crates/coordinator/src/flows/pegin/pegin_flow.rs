use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::Parity::Even;
use bitcoin::secp256k1::XOnlyPublicKey;
use bitcoin::{PublicKey, Txid};
use common_bitvmx::bitvmx_types::{
    ACCEPT_PEGIN_TX, BitVmxProtocolId, BtcTxSPVProof, CommsAddress, IncomingBitVMXApiMessages,
    OPERATOR_TAKE_TX, OPERATOR_WON_TX, ParticipantRole, PeginAcceptedMessage, PubKeyHash,
    TransactionStatus, VariableTypes, accept_pegin_protocol_id, build_communication_data,
};
use common_broker::broker::BitVmxBrokerClientApi;
use common_core::types::{CommitteeId, TxIdParser};
use common_runtime::runtime_sync::RuntimeSync;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, trace, warn};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::{
    GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    P2PAddressParser, RequestPeginInput,
};
use union_contracts::bindings::pegin_manager::PeginManager::{PeginAccepted, PeginRequested};
use uuid::Uuid;

use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::flows::common::{COMM_KEY_INDEX, FlowId, Signaling};
use crate::store::{CoordinatorStoreApi, StoreKey};

const PEGIN_REQUEST: &str = "pegin_request";
const PEGIN_ACCEPTED_VAR_NAME: &str = "PeginAccepted";
const PROGRAM_TYPE_ACCEPT_PEGIN: &str = "accept_pegin";

/// Derive the pegin flow id from the request-pegin BTC txid.
#[must_use]
pub(crate) fn flow_id_from_request_pegin_txid(request_pegin_txid: Txid) -> FlowId {
    FlowId::from_tx("pegin_flow", request_pegin_txid.to_byte_array().as_slice())
}

/// Steps for the pegin state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum Steps {
    // Initial state when Bitcoin pegin transaction is found
    #[default]
    PeginTransactionFound,
    // Request SPV proof for the pegin request (to call requestPegin)
    RequestPeginSpvProof,
    // Wait for the confirmed RSK PeginRequested event after submitting requestPegin.
    WaitPeginRequested,
    // Authoritative checkpoint: entered after confirmed PeginRequested establishes
    // the canonical requestPegin result.
    GetCommInfoAuthoritativeCheckpoint,
    // Send pegin request to BitVMX and setup
    PreparePeginSetup,
    // Request BitVMX operator take transaction info for prover members
    RequestOperatorTakeTransactionInfo,
    // Request BitVMX operator won transaction info for prover members
    RequestOperatorWonTransactionInfo,
    // Add operator take tx hash to contracts after BitVMX accepts
    AddOperatorTakeHash,
    // Wait until every operator take tx hash has been registered on Rootstock
    WaitAllOperatorTakeTxidsAdded,
    // All-converge checkpoint: entered after AllOperatorTakeTxidsAdded proves every
    // required operator take txid was registered. Wait for the accept-pegin signing subflow.
    WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint,
    // All-converge checkpoint: entered after the accept-pegin signing subflow completes.
    // Dispatch the fully signed accept-pegin Bitcoin transaction.
    DispatchAcceptPeginTransactionAllConvergeCheckpoint,
    // Confirm accept pegin transaction
    ConfirmAcceptPeginTransaction,
    // Request SPV proof for the accept pegin (to call acceptPegin)
    RequestAcceptPeginSpvProof,
    // Accept the pegin on RSK
    AcceptPegin,
    // Terminal state after confirmed RSK PeginAccepted establishes the acceptPegin result.
    Done,
    Failed,
}

impl Steps {
    pub(crate) fn allows_fast_forward_to_pegin_accepted(self) -> bool {
        matches!(
            self,
            Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint
                | Steps::ConfirmAcceptPeginTransaction
                | Steps::RequestAcceptPeginSpvProof
                | Steps::AcceptPegin
        )
    }
}

/// Data passed between steps in the pegin flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StepData {
    // Initial Bitcoin transaction found
    PeginTransactionFound,
    // SPV proof for request pegin
    RequestPeginSpvProof(BtcTxSPVProof),
    // Retry request pegin without state transition data
    RetryRequestPegin,
    // Pegin requested
    PeginRequested(PeginRequested),
    // Communication info
    CommInfo(CommsAddress),
    // BitVMX pegin accepted
    BitvmxPeginAccepted(PeginAcceptedMessage),
    // Named transaction information returned by BitVMX
    TransactionInfo { tx_name: String, txid: Txid },
    // Local operator take tx hash submitted
    OperatorTakeHashAdded,
    // All operator take tx hashes added
    AllOperatorTakeTxidsAdded,
    // Accept pegin signatures are ready
    AcceptPeginSignaturesReady,
    // Accept pegin transaction has been dispatched
    AcceptPeginTransactionDispatched,
    // Accept pegin transaction confirmed
    AcceptPeginTransactionConfirmed(TransactionStatus),
    // SPV proof for accept pegin
    AcceptPeginSpvProof(BtcTxSPVProof),
    // Retry accept pegin without state transition
    RetryAcceptPegin,
    // Pegin accepted
    PeginAccepted(PeginAccepted),
}

/// Data structure used to send pegin request information to `BitVMX`
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PeginRequestMessage {
    pub txid: Txid,
    pub amount: u64,
    pub accept_pegin_sighash: Vec<u8>,
    pub take_aggregated_key: PublicKey,
    pub operator_indexes: Vec<usize>,
    pub slot_index: u64,
    pub committee_id: Uuid,
    pub rootstock_address: String,
    pub reimbursement_pubkey: PublicKey,
}

/// Context for the pegin flow state machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlowContext {
    pub flow_id: FlowId,
    /// Canonical on-chain identifier this flow tracks. Set at flow
    /// creation from the `PeginTransactionFound` event.
    pub request_pegin_btc_tx_id: Txid,
    pub step: Steps,

    /// `BitVMX` program id for the accept-pegin program. `None` until
    /// `PeginRequested` arrives, since the id is derived from
    /// `(committee_id, slot_index)`. Used at every `BitVMX` message boundary
    /// (`Setup`, `SetVar`, `GetTransactionInfoByName`,
    /// `DispatchTransactionName`, `GetTransaction`) and matches
    /// `get_accept_pegin_pid` on the `BitVMX` side.
    pub bitvmx_protocol_id: Option<BitVmxProtocolId>,

    pub request_pegin_btc_tx_status: Option<TransactionStatus>,
    pub request_pegin_spv_proof: Option<BtcTxSPVProof>,
    pub pegin_requested: Option<PeginRequested>,
    pub my_p2p_address: Option<CommsAddress>,
    pub committee_output: Option<GetCommitteeOutput>,
    pub bitvmx_pegin_accepted: Option<PeginAcceptedMessage>,
    pub operator_take_txid: Option<Txid>,
    pub operator_won_txid: Option<Txid>,
    pub accept_pegin_spv_proof: Option<BtcTxSPVProof>,
    pub accept_pegin_tx_status: Option<TransactionStatus>,
    pub pegin_accepted: Option<PeginAccepted>,
    pub op_role: Option<ParticipantRole>,
}

/// Serializable state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct State {
    pub flow_id: FlowId,
    /// Pre-formatted display id `{flow_id} ({request_pegin_btc_tx_id})`
    /// for log lines. Not persisted — re-computed at construction and on
    /// `from_saved_state` via `build_log_id`.
    #[serde(skip)]
    pub log_id: String,
    pub ctx: FlowContext,
    /// When this flow was first created. `None` for flows persisted before
    /// this field existed (they pre-date the change and we can't backfill).
    // TODO: once all v0.4.0 flows have been migrated through this version and
    // no on-disk record lacks `created_at`, drop both `#[serde(default)]` and
    // the `Option` wrapper (the field becomes a required `DateTime<Utc>`).
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl State {
    fn build_log_id(&self) -> String {
        format!("{} ({})", self.flow_id, self.ctx.request_pegin_btc_tx_id)
    }
}

/// State machine for handling pegin flow
pub(crate) struct PeginFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    state: State,
    store: Rc<S>,
    signaling: Rc<Signaling>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
}

impl<CG, BC, S> PeginFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    /// Create a new pegin flow from `PeginTransactionFound`
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        btc_tx_id: Txid,
        flow_id: FlowId,
        store: Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        let mut state = State {
            flow_id,
            log_id: String::new(),
            ctx: FlowContext {
                flow_id,
                request_pegin_btc_tx_id: btc_tx_id,
                step: Steps::PeginTransactionFound,
                bitvmx_protocol_id: None,
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
            created_at: Some(chrono::Utc::now()),
        };
        state.log_id = state.build_log_id();
        info!("PeginFlow created");

        Self { contracts, rt_sync, bitvmx_broker, state, store, signaling, native_bridge_verifier }
    }

    pub(crate) fn from_saved_state(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        mut state: State,
        store: Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        state.log_id = state.build_log_id();
        Self { contracts, rt_sync, bitvmx_broker, state, store, signaling, native_bridge_verifier }
    }

    fn persist_state(&self) -> Result<()> {
        debug!("Persisting state for step: {:?}", self.state.ctx.step);
        self.store.save_flow(&StoreKey::PeginFlow(self.state.flow_id.value()), self.state.clone())
    }

    /// Start the next step and log the transition
    pub(crate) fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.ctx.step;
        self.state.ctx.step = next_step;

        debug!("{} -> {}", format_step(previous_step), format_step(next_step));

        if next_step == Steps::AcceptPegin {
            self.persist_state()?;
        }

        // Execute the entry action for the new state
        match next_step {
            Steps::PeginTransactionFound => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::RequestPeginSpvProof => {
                self.request_pegin_spv_proof()?;
            }
            Steps::WaitPeginRequested => {
                info!("Waiting for PeginRequested event");
            }
            Steps::GetCommInfoAuthoritativeCheckpoint => {
                self.request_bitvmx_comm_info()?;
            }
            Steps::PreparePeginSetup => {
                self.prepare_pegin_setup()?;
            }
            Steps::RequestOperatorTakeTransactionInfo => {
                info!("Requesting operator take transaction info from BitVMX");
                self.request_operator_take_transaction_info()?;
            }
            Steps::RequestOperatorWonTransactionInfo => {
                info!("Requesting operator won transaction info from BitVMX");
                self.request_operator_won_transaction_info()?;
            }
            Steps::AddOperatorTakeHash => {
                self.add_operator_take_hash()?;
                self.complete_step(&StepData::OperatorTakeHashAdded)?;
            }
            Steps::WaitAllOperatorTakeTxidsAdded => {
                info!("Waiting for AllOperatorTakeTxidsAdded");
            }
            Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint => {
                info!("Waiting for signatures to be ready to dispatch transaction");
            }
            Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint => {
                self.dispatch_transaction()?;
                self.complete_step(&StepData::AcceptPeginTransactionDispatched)?;
            }
            Steps::ConfirmAcceptPeginTransaction => {
                // Transaction status will be polled via TickScheduler in the processor
                // to ensure the transaction has time to be broadcast before querying
                info!(
                    "Waiting for AcceptPegin Bitcoin confirmations: accept_tx_id={:?}",
                    self.get_accept_pegin_txid_from_bitvmx_var()
                );
            }
            Steps::RequestAcceptPeginSpvProof => {
                info!(
                    "Requesting SPV proof for accept pegin: accept_tx_id={:?}",
                    self.get_accept_pegin_txid_from_bitvmx_var()
                );
                self.request_spv_proof()?;
            }
            Steps::AcceptPegin => {
                info!("Accepting pegin");
                let spv_proof = self
                    .state
                    .ctx
                    .accept_pegin_spv_proof
                    .as_ref()
                    .ok_or_else(|| anyhow!("SPV proof not available for pegin acceptance"))?;
                self.accept_pegin(spv_proof)?;
            }
            Steps::Done => {
                self.send_pegin_accepted_to_bitvmx()?;
                self.write_completion_marker()?;
                metrics::counter!("union_flows_completed_total", "type" => "pegin").increment(1);
                match self.extract_pegin_amount() {
                    Ok(sats) => metrics::counter!("union_pegin_amount_sats_total").increment(sats),
                    Err(e) => warn!("Pegin completed but BTC amount unavailable for metric: {e:#}"),
                }
                info!("Done");
            }
            Steps::Failed => {
                info!("Failed");
            }
        }

        // Persist state after successful step completion. Some entry actions complete
        // another step synchronously and persist that newer state.
        if self.state.ctx.step == next_step {
            self.persist_state()?;
        }

        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub(crate) fn complete_step(&mut self, data: &StepData) -> Result<()> {
        let current_step = self.state.ctx.step;

        debug!("Completing step {} with data: {:?}", format_step(current_step), data);

        // Process data and determine next state
        let next_step = self.process_step_data(current_step, data)?;

        // Transition to the next state
        self.start_step(next_step)?;

        Ok(())
    }

    /// Process the current step data and determine the next state
    fn process_step_data(&mut self, current_step: Steps, data: &StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::PeginTransactionFound, StepData::PeginTransactionFound) => {
                Ok(Steps::RequestPeginSpvProof)
            }
            (Steps::RequestPeginSpvProof, StepData::RequestPeginSpvProof(spv_proof)) => {
                self.state.ctx.request_pegin_spv_proof = Some(spv_proof.clone());
                self.persist_state()?;
                self.request_pegin(spv_proof)?;
                Ok(Steps::WaitPeginRequested)
            }
            (Steps::RequestPeginSpvProof, StepData::RetryRequestPegin) => {
                self.retry_request_pegin_step()
            }
            (Steps::WaitPeginRequested, StepData::PeginRequested(pegin_requested)) => {
                self.state.ctx.pegin_requested = Some(pegin_requested.clone());
                // committee_id and slotId are now known: derive the BitVMX
                // protocol id (must match `get_accept_pegin_pid` on the
                // BitVMX side, used in dispute_core lookups).
                let committee_id: CommitteeId = pegin_requested.committeeId.into();
                let committee_uuid = Uuid::from_u128(*committee_id);
                let slot_index = usize::try_from(pegin_requested.streamPosition.slotId)
                    .map_err(|_| anyhow!("Slot ID too large for usize"))?;
                self.state.ctx.bitvmx_protocol_id =
                    Some(accept_pegin_protocol_id(committee_uuid, slot_index));
                Ok(Steps::GetCommInfoAuthoritativeCheckpoint)
            }
            (Steps::GetCommInfoAuthoritativeCheckpoint, StepData::CommInfo(comm_info)) => {
                self.state.ctx.my_p2p_address = Some(comm_info.clone());
                Ok(Steps::PreparePeginSetup)
            }
            (Steps::PreparePeginSetup, StepData::BitvmxPeginAccepted(accepted)) => {
                self.state.ctx.bitvmx_pegin_accepted = Some(accepted.clone());
                if self.try_get_op_role()? == ParticipantRole::Prover {
                    Ok(Steps::RequestOperatorTakeTransactionInfo)
                } else {
                    Ok(Steps::WaitAllOperatorTakeTxidsAdded)
                }
            }
            (
                Steps::RequestOperatorTakeTransactionInfo,
                StepData::TransactionInfo { tx_name, txid },
            ) => self.handle_operator_take_transaction_info(tx_name, *txid),
            (
                Steps::RequestOperatorWonTransactionInfo,
                StepData::TransactionInfo { tx_name, txid },
            ) => self.handle_operator_won_transaction_info(tx_name, *txid),
            (Steps::AddOperatorTakeHash, StepData::OperatorTakeHashAdded) => {
                Ok(Steps::WaitAllOperatorTakeTxidsAdded)
            }
            (Steps::WaitAllOperatorTakeTxidsAdded, StepData::AllOperatorTakeTxidsAdded) => {
                Ok(Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint)
            }
            (
                Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint,
                StepData::AcceptPeginSignaturesReady,
            ) => Ok(Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint),
            (
                Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint,
                StepData::AcceptPeginTransactionDispatched,
            ) => Ok(Steps::ConfirmAcceptPeginTransaction),
            (
                Steps::ConfirmAcceptPeginTransaction,
                StepData::AcceptPeginTransactionConfirmed(tx_status),
            ) => self.confirm_accept_pegin_transaction(tx_status),
            (Steps::RequestAcceptPeginSpvProof, StepData::AcceptPeginSpvProof(spv_proof)) => {
                info!("Received SPV proof for accept pegin");
                trace!("SPV Proof data: {spv_proof:?}");
                self.state.ctx.accept_pegin_spv_proof = Some(spv_proof.clone());
                Ok(Steps::AcceptPegin)
            }
            (Steps::AcceptPegin, StepData::RetryAcceptPegin) => {
                info!("Retrying accept pegin");
                Ok(Steps::AcceptPegin)
            }
            (step, StepData::PeginAccepted(pegin_accepted))
                if step.allows_fast_forward_to_pegin_accepted() =>
            {
                self.complete_with_confirmed_pegin_acceptance(pegin_accepted)
            }
            _ => Err(anyhow!("Invalid state transition: {current_step:?} with data {data:?}")),
        }
    }

    fn complete_with_confirmed_pegin_acceptance(
        &mut self,
        pegin_accepted: &PeginAccepted,
    ) -> Result<Steps> {
        let expected_txid = self
            .get_accept_pegin_txid_from_bitvmx_var()
            .ok_or_else(|| anyhow!("Expected accept pegin txid not found"))?;
        let accepted_txid = TxIdParser::fb_32_to_txid(pegin_accepted.acceptPeginTxid);

        if accepted_txid != expected_txid {
            bail!("PeginAccepted txid mismatch: got {accepted_txid:?}, expected {expected_txid:?}");
        }

        trace!("PeginAccepted data: {pegin_accepted:?}");
        self.state.ctx.pegin_accepted = Some(pegin_accepted.clone());

        Ok(Steps::Done)
    }

    fn prepare_pegin_setup(&mut self) -> Result<()> {
        info!("Preparing pegin setup for BitVMX");

        let pegin_requested = self
            .state
            .ctx
            .pegin_requested
            .as_ref()
            .ok_or_else(|| anyhow!("PeginRequested data not available"))?;
        let committee_id: CommitteeId = pegin_requested.committeeId.into();

        self.send_pegin_request_to_bitvmx(&committee_id)?;
        self.send_setup_to_bitvmx(&committee_id)?;
        Ok(())
    }

    fn send_pegin_request_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!("Sending pegin request to BitVMX");

        let committee_output = self.get_committee_output(committee_id.clone())?;
        self.state.ctx.committee_output = Some(committee_output.clone());
        self.state.ctx.op_role = Some(self.calc_op_role()?);

        let pegin_requested = self
            .state
            .ctx
            .pegin_requested
            .as_ref()
            .ok_or_else(|| anyhow!("PeginRequested data not available"))?;
        let amount = self.extract_pegin_amount()?;
        let pegin_request =
            Self::build_pegin_request_message(pegin_requested, &committee_output, amount)?;

        let msg = IncomingBitVMXApiMessages::SetVar(
            self.bitvmx_protocol_id()?.value(),
            PEGIN_REQUEST.to_string(),
            VariableTypes::String(serde_json::to_string(&pegin_request)?),
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn send_setup_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!("Sending setup to BitVMX");

        let committee_addresses = self.get_committee_addresses(committee_id)?;
        let committee_pubkey_hashes = self.get_committee_pubkey_hashes(committee_id)?;

        let comms_addresses = build_communication_data(
            &self
                .state
                .ctx
                .my_p2p_address
                .as_ref()
                .ok_or_else(|| anyhow!("P2P address not available for setup"))?
                .address
                .to_string(),
            &committee_addresses,
            &committee_pubkey_hashes,
        )?;

        let msg = IncomingBitVMXApiMessages::Setup(
            self.bitvmx_protocol_id()?.value(),
            PROGRAM_TYPE_ACCEPT_PEGIN.to_string(),
            comms_addresses,
            0, // No leader
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    pub(crate) fn add_operator_take_hash(&self) -> Result<()> {
        if self.try_get_op_role()? != ParticipantRole::Prover {
            info!("Skipping add_operator_take_hash for verifier");
            return Ok(());
        }

        let pegin_accepted = self
            .get_bitvmx_pegin_accepted()
            .ok_or_else(|| anyhow!("BitVMX pegin accepted message not found"))?;

        debug!(
            "Adding operator (prover) take tx hash: accept_pegin_txid={}",
            pegin_accepted.accept_pegin_txid
        );

        let take_tx_hash = self
            .state
            .ctx
            .operator_take_txid
            .ok_or_else(|| anyhow!("operator_take_txid missing in pegin flow state"))?;

        let won_tx_hash = self
            .state
            .ctx
            .operator_won_txid
            .ok_or_else(|| anyhow!("operator_won_txid missing in pegin flow state"))?;

        let input = transaction_dispatcher::types::AddOperatorTakeTxHashInput {
            accept_pegin_tx_hash: pegin_accepted.accept_pegin_txid,
            take_tx_hash,
            won_tx_hash,
        };

        self.rt_sync.run(async { self.contracts.add_operator_take_tx_hash(input).await })?;

        Ok(())
    }

    fn request_operator_take_transaction_info(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransactionInfoByName(
            self.bitvmx_protocol_id()?.value(),
            self.operator_take_transaction_name()?,
        ))?;

        Ok(())
    }

    fn request_operator_won_transaction_info(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransactionInfoByName(
            self.bitvmx_protocol_id()?.value(),
            self.operator_won_transaction_name()?,
        ))?;

        Ok(())
    }

    fn dispatch_transaction(&self) -> Result<()> {
        info!("Dispatching {}", ACCEPT_PEGIN_TX);
        let msg = IncomingBitVMXApiMessages::DispatchTransactionName(
            self.bitvmx_protocol_id()?.value(),
            ACCEPT_PEGIN_TX.to_string(),
        );
        self.send_bitvmx_msg(msg)?;
        Ok(())
    }

    fn accept_pegin(&self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        debug!("Accepting pegin with SPV proof: {spv_proof:?}");

        let input: RequestPeginInput = spv_proof.clone().into();

        invoke_contract_safe(
            &self.rt_sync,
            "acceptPegin",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.accept_pegin(input).await },
        )
        .context("Failed to accept pegin with provided SPV proof")?;

        Ok(())
    }

    fn request_pegin(&self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        debug!("Requesting pegin with SPV proof: {spv_proof:?}");

        let input: RequestPeginInput = spv_proof.clone().into();

        match invoke_contract_safe(
            &self.rt_sync,
            "requestPegin",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.request_pegin(input).await },
        ) {
            Ok(_) => Ok(()),
            Err(DomainErrors::PeginAlreadyRequested(msg)) => {
                info!("Pegin already requested, continuing: {msg}");
                Ok(())
            }
            Err(err) => Err(err).context("Failed to request pegin with provided SPV proof"),
        }?;

        Ok(())
    }

    fn send_pegin_accepted_to_bitvmx(&self) -> Result<()> {
        let pegin_accepted = self
            .state
            .ctx
            .pegin_accepted
            .as_ref()
            .ok_or_else(|| anyhow!("PeginAccepted data not available"))?;

        debug!("Notifying pegin accepted to BitVMX");
        let data = serde_json::to_string(&pegin_accepted)?;
        let msg = IncomingBitVMXApiMessages::SetVar(
            self.bitvmx_protocol_id()?.value(),
            PEGIN_ACCEPTED_VAR_NAME.to_string(),
            VariableTypes::String(data),
        );

        self.send_bitvmx_msg(msg)
    }

    fn write_completion_marker(&self) -> Result<()> {
        let payload = json!({
            "request_pegin_btc_tx_id": self.state.ctx.request_pegin_btc_tx_id.to_string(),
            "request_pegin_txid": self.state.ctx.pegin_requested.as_ref().map(|event| format!("{:#066x}", event.requestPeginTxid)),
            "accept_pegin_txid": self.state.ctx.pegin_accepted.as_ref().map(|event| format!("{:#066x}", event.acceptPeginTxid)),
            "committee_id": self.state.ctx.pegin_requested.as_ref().map(|event| event.committeeId.to_string()),
            "slot_id": self.state.ctx.pegin_requested.as_ref().map(|event| event.streamPosition.slotId),
            "rsk_destination_address": self.state.ctx.pegin_accepted.as_ref().map(|event| event.rskDestinationAddress.to_string()),
            "rbtc_amount": self.state.ctx.pegin_accepted.as_ref().map(|event| event.rbtcAmount.to_string()),
        });

        // signal_done currently keys completion markers by a Uuid; reuse the
        // BitVMX protocol id (always set by the time we reach Done).
        self.signaling.signal_done("pegin", self.bitvmx_protocol_id()?.value(), &payload)
    }

    fn extract_pegin_amount(&self) -> Result<u64> {
        let spv_proof = self
            .state
            .ctx
            .request_pegin_spv_proof
            .as_ref()
            .ok_or_else(|| anyhow!("Request pegin SPV proof not available"))?;

        spv_proof
            .tx
            .output
            .first()
            .map(|o| o.value.to_sat())
            .ok_or_else(|| anyhow!("Request pegin BTC transaction has no outputs"))
    }

    fn request_pegin_spv_proof(&self) -> Result<()> {
        let btc_tx_id = self.state.ctx.request_pegin_btc_tx_id;

        info!("Requesting SPV proof: btc_tx_id={btc_tx_id}");
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(btc_tx_id))?;
        Ok(())
    }

    fn build_pegin_request_message(
        event: &PeginRequested,
        committee_output: &GetCommitteeOutput,
        amount: u64,
    ) -> Result<PeginRequestMessage> {
        debug!("Building PeginRequestMessage for BitVMX from PeginRequested event");

        let committee_id = Uuid::from_u128(event.committeeId);
        let operator_indexes = Self::build_operator_indexes(committee_output);
        let slot_index = event.streamPosition.slotId;

        let checksum_address = event.requestPeginInfo.rskDestinationAddress.to_checksum(None);
        let rootstock_address = checksum_address
            .get(2..)
            .ok_or_else(|| anyhow!("RSK address checksum too short"))?
            .to_string();

        let accept_pegin_sighash = event.acceptPeginSignatureMessage.to_vec();
        let take_aggregated_key = Self::build_take_aggregated_key(committee_output)?;
        let reimbursement_pubkey = Self::build_reimbursement_pubkey(event)?;
        let txid = TxIdParser::fb_32_to_txid(event.requestPeginTxid);

        Ok(PeginRequestMessage {
            txid,
            amount,
            accept_pegin_sighash,
            take_aggregated_key,
            operator_indexes,
            slot_index,
            committee_id,
            rootstock_address,
            reimbursement_pubkey,
        })
    }

    fn build_operator_indexes(committee_response: &GetCommitteeOutput) -> Vec<usize> {
        let operator_role: u8 = ParticipantRole::Prover.into();
        let mut operator_indexes = Vec::new();

        for (i, member) in committee_response.committee.members.iter().enumerate() {
            if member.role == operator_role {
                operator_indexes.push(i);
            }
        }

        operator_indexes
    }

    fn build_take_aggregated_key(committee_response: &GetCommitteeOutput) -> Result<PublicKey> {
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn build_reimbursement_pubkey(event: &PeginRequested) -> Result<PublicKey> {
        let reimbursement_xonly_key =
            XOnlyPublicKey::from_slice(event.requestPeginInfo.btcReimbursementPubKey.as_slice())
                .context("Failed to parse reimbursement public key from pegin event")?;
        let reimbursement_secp_key = reimbursement_xonly_key.public_key(Even);
        Ok(PublicKey::new(reimbursement_secp_key))
    }

    fn get_committee_output(&mut self, committee_id: CommitteeId) -> Result<GetCommitteeOutput> {
        let committee_response = self.rt_sync.run(async {
            self.contracts.get_committee(GetCommitteeInput { committee_id }).await
        })?;
        Ok(committee_response)
    }

    fn get_committee_addresses(&self, committee_id: &CommitteeId) -> Result<Vec<String>> {
        let input = GetCommunicationDataInput {
            committee_id: committee_id.clone(),
            member_address: self.contracts.my_address().into(),
        };
        let communication_data_response = self
            .rt_sync
            .run(async { self.contracts.get_committee_communication_data(input).await })?;

        let committee_addresses = communication_data_response
            .communication_data
            .into_iter()
            .map(|comm_data| {
                P2PAddressParser::socket_addr_from_contracts(&comm_data)
                    .map(|opt_addr| opt_addr.map(|addr| addr.to_string()).unwrap_or_default())
                    .context("Failed to convert communication data to P2P address")
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(committee_addresses)
    }

    fn get_committee_pubkey_hashes(&self, committee_id: &CommitteeId) -> Result<Vec<PubKeyHash>> {
        let committee_input = GetCommitteeInput { committee_id: committee_id.clone() };
        let committee_response =
            self.rt_sync.run(async { self.contracts.get_committee(committee_input).await })?;

        let mut pubkey_hashes = Vec::new();

        for member in committee_response.committee.members {
            let keys_input = GetMemberPublicKeysInput { member_address: member.memberAddress };

            let keys_response = self
                .rt_sync
                .run(async { self.contracts.get_member_public_keys(keys_input).await })?;

            let key_str = keys_response.public_keys.get(COMM_KEY_INDEX).with_context(|| {
                format!("Communication key not found for member {}", member.memberAddress)
            })?;

            debug!(
                "Member pubkey_hash: address={}, pubkey_hash={:?}",
                member.memberAddress, key_str
            );
            pubkey_hashes.push(key_str.clone());
        }

        Ok(pubkey_hashes)
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        info!("Requesting BitVMX comm info");
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    pub(crate) fn request_transaction_status(&self) -> Result<()> {
        let tx_id = self
            .get_accept_pegin_txid_from_bitvmx_var()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;
        info!("Requesting transaction status: tx_id={:?}", tx_id);
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(
            self.bitvmx_protocol_id()?.value(),
            tx_id,
        ))?;
        Ok(())
    }

    pub(crate) fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .get_accept_pegin_txid_from_bitvmx_var()
            .ok_or_else(|| anyhow!("Expected accept pegin tx_id not found"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))?;
        Ok(())
    }

    /// Calculates the operator role based on the committee members and its own address
    pub(crate) fn calc_op_role(&self) -> Result<ParticipantRole> {
        let Some(get_committee_output) = &self.state.ctx.committee_output else {
            bail!("Committee output not found");
        };

        let my_addr: alloy_primitives::Address = self.contracts.my_address().into();
        let Some(member) =
            get_committee_output.committee.members.iter().find(|e| e.memberAddress == my_addr)
        else {
            bail!("Address not found in committee members");
        };

        member.role.try_into().context("Failed to convert u8 role to ParticipantRole")
    }

    fn try_get_op_role(&self) -> Result<ParticipantRole> {
        if let Some(role) = &self.state.ctx.op_role {
            return Ok(role.clone());
        }

        bail!("Operator role not found in context")
    }

    fn my_committee_index(&self) -> Result<usize> {
        let Some(get_committee_output) = &self.state.ctx.committee_output else {
            bail!("Committee output not found");
        };

        let my_addr: alloy_primitives::Address = self.contracts.my_address().into();
        get_committee_output
            .committee
            .members
            .iter()
            .position(|member| member.memberAddress == my_addr)
            .context("Address not found in committee members")
    }

    fn operator_take_transaction_name(&self) -> Result<String> {
        let member_index = self.my_committee_index()?;
        Ok(indexed_name(OPERATOR_TAKE_TX, member_index))
    }

    fn operator_won_transaction_name(&self) -> Result<String> {
        let member_index = self.my_committee_index()?;
        Ok(indexed_name(OPERATOR_WON_TX, member_index))
    }

    fn retry_request_pegin_step(&mut self) -> Result<Steps> {
        let spv_proof = self
            .state
            .ctx
            .request_pegin_spv_proof
            .as_ref()
            .ok_or_else(|| anyhow!("SPV proof not available for pegin request"))?;
        self.request_pegin(spv_proof)?;
        Ok(Steps::WaitPeginRequested)
    }

    fn handle_operator_take_transaction_info(
        &mut self,
        tx_name: &str,
        txid: Txid,
    ) -> Result<Steps> {
        let expected_tx_name = self.operator_take_transaction_name()?;
        if tx_name != expected_tx_name {
            bail!(
                "Unexpected transaction info in step {:?}: got {}, expected {}",
                Steps::RequestOperatorTakeTransactionInfo,
                tx_name,
                expected_tx_name
            );
        }
        self.state.ctx.operator_take_txid = Some(txid);
        Ok(Steps::RequestOperatorWonTransactionInfo)
    }

    fn handle_operator_won_transaction_info(&mut self, tx_name: &str, txid: Txid) -> Result<Steps> {
        let expected_tx_name = self.operator_won_transaction_name()?;
        if tx_name != expected_tx_name {
            bail!(
                "Unexpected transaction info in step {:?}: got {}, expected {}",
                Steps::RequestOperatorWonTransactionInfo,
                tx_name,
                expected_tx_name
            );
        }
        self.state.ctx.operator_won_txid = Some(txid);
        Ok(Steps::AddOperatorTakeHash)
    }

    fn confirm_accept_pegin_transaction(&mut self, tx_status: &TransactionStatus) -> Result<Steps> {
        info!("Transaction confirmed: tx_id={:?}", tx_status.tx_id);
        trace!("Transaction status data: {tx_status:?}");
        let expected_tx_id = self
            .get_accept_pegin_txid_from_bitvmx_var()
            .ok_or_else(|| anyhow!("Expected accept pegin txid not found"))?;
        if tx_status.tx_id != expected_tx_id {
            bail!(
                "Pegin {} transaction status txId mismatch: got {:?}, expected {:?}",
                self.flow_id(),
                tx_status.tx_id,
                expected_tx_id
            );
        }
        self.state.ctx.accept_pegin_tx_status = Some(tx_status.clone());
        Ok(Steps::RequestAcceptPeginSpvProof)
    }

    pub(crate) fn get_accept_pegin_txid_from_bitvmx_var(&self) -> Option<Txid> {
        self.state.ctx.bitvmx_pegin_accepted.as_ref().map(|accepted| accepted.accept_pegin_txid)
    }

    /// `BitVMX` program id for the accept-pegin program. Populated once
    /// `PeginRequested` has been processed; calling before that is a
    /// state-machine bug.
    fn bitvmx_protocol_id(&self) -> Result<BitVmxProtocolId> {
        self.state.ctx.bitvmx_protocol_id.ok_or_else(|| {
            anyhow!(
                "Pegin {} bitvmx_protocol_id not yet set at step {:?}",
                self.flow_id(),
                self.state.ctx.step
            )
        })
    }

    /// Convenience for callers that need to handle the not-yet-set state.
    pub(crate) fn bitvmx_protocol_id_opt(&self) -> Option<BitVmxProtocolId> {
        self.state.ctx.bitvmx_protocol_id
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self.state.ctx.step, Steps::Done | Steps::Failed)
    }

    pub(crate) fn mark_failed(&mut self, reason: &str) -> Result<()> {
        info!("Marking as failed: {reason}");
        self.start_step(Steps::Failed)
    }

    /// Get the flow id (a `Uuid` derived from `request_pegin_btc_tx_id`).
    pub(crate) fn flow_id(&self) -> FlowId {
        self.state.flow_id
    }

    /// Pre-formatted display id for log lines.
    pub(crate) fn log_id(&self) -> &str {
        &self.state.log_id
    }

    /// Get the canonical on-chain identifier (the request-pegin BTC txid).
    pub(crate) fn request_pegin_btc_tx_id(&self) -> Txid {
        self.state.ctx.request_pegin_btc_tx_id
    }

    /// Get the current step
    pub(crate) fn current_step(&self) -> Steps {
        self.state.ctx.step
    }

    pub(crate) fn get_flow_details(&self) -> crate::event_processor::FlowDetails {
        crate::event_processor::FlowDetails {
            kind: crate::types::FlowKind::Pegin,
            id: self.flow_id().to_string(),
            step: format!("{:?}", self.current_step()),
            created_at: self.state.created_at,
        }
    }

    /// Get the state for debugging
    pub(crate) fn get_state(&self) -> &State {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn get_state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Get the `BitVMX` pegin accepted message if available
    pub(crate) fn get_bitvmx_pegin_accepted(&self) -> Option<&PeginAcceptedMessage> {
        self.state.ctx.bitvmx_pegin_accepted.as_ref()
    }
}

fn indexed_name(prefix: &str, index: usize) -> String {
    format!("{prefix}_{index}")
}

/// Helper function to format step names for logging
fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::PeginTransactionFound => "PeginTransactionFound",
        Steps::RequestPeginSpvProof => "RequestPeginSpvProof",
        Steps::WaitPeginRequested => "WaitPeginRequested",
        Steps::GetCommInfoAuthoritativeCheckpoint => "GetCommInfoAuthoritativeCheckpoint",
        Steps::PreparePeginSetup => "PreparePeginSetup",
        Steps::RequestOperatorTakeTransactionInfo => "RequestOperatorTakeTransactionInfo",
        Steps::RequestOperatorWonTransactionInfo => "RequestOperatorWonTransactionInfo",
        Steps::AddOperatorTakeHash => "AddOperatorTakeHash",
        Steps::WaitAllOperatorTakeTxidsAdded => "WaitAllOperatorTakeTxidsAdded",
        Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint => {
            "WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint"
        }
        Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint => {
            "DispatchAcceptPeginTransactionAllConvergeCheckpoint"
        }
        Steps::ConfirmAcceptPeginTransaction => "ConfirmAcceptPeginTransaction",
        Steps::RequestAcceptPeginSpvProof => "RequestAcceptPeginSpvProof",
        Steps::AcceptPegin => "AcceptPegin",
        Steps::Done => "Done",
        Steps::Failed => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address as AlloyAddress, Bytes, FixedBytes, U256, Uint};
    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash;
    use bitcoin::transaction::Version;
    use bitcoin::{Transaction, Txid};
    use common_bitvmx::bitvmx_types::{
        IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, PeginAcceptedMessage,
    };
    use common_broker::broker::MockBrokerClientApi;
    use common_core::types::Address as CommonAddress;
    use common_runtime::runtime_sync::RuntimeSync;
    use mockall::predicate::*;
    use musig2::PubNonce;
    use musig2::secp::MaybeScalar;
    use primitive_types::H160;
    use transaction_dispatcher::types::GetCommitteeOutput;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::Committee;
    use union_contracts::bindings::pegin_manager::PeginManager::StreamPosition;
    use uuid::Uuid;

    use super::*;
    use crate::store::{CoordinatorStore, MockCoordinatorStoreApi, TestStorePath};

    type MockPeginFlow = PeginFlow<
        crate::coordinator::tests::MockRskContractsGatewayApi,
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        MockCoordinatorStoreApi,
    >;

    type StoreBackedPeginFlow = PeginFlow<
        crate::coordinator::tests::MockRskContractsGatewayApi,
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>,
        CoordinatorStore,
    >;

    const ROLE_PROVER: u8 = 1;
    const ROLE_VERIFIER: u8 = 2;

    fn test_address(bytes: [u8; 20]) -> CommonAddress {
        CommonAddress::from(H160::from(bytes))
    }

    fn test_txid(bytes: [u8; 32]) -> Txid {
        Txid::from_raw_hash(
            bitcoin::hashes::sha256d::Hash::from_slice(&bytes).expect("Invalid hash"),
        )
    }

    fn default_pub_nonce() -> PubNonce {
        "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93"
            .parse::<PubNonce>()
            .expect("Invalid PubNonce")
    }

    fn default_pegin_accepted_message(accept_pegin_txid: Txid) -> PeginAcceptedMessage {
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

    fn default_pegin_accepted_event(accept_pegin_txid: Txid) -> PeginAccepted {
        PeginAccepted {
            blockHash: FixedBytes::<32>::from([0u8; 32]),
            acceptPeginTxid: TxIdParser::txid_to_fb_32(accept_pegin_txid),
            requestPeginTxid: FixedBytes::<32>::from([1u8; 32]),
            vout: 0,
            streamPosition: StreamPosition {
                streamId: 0,
                packetNumber: 0,
                slotId: 0,
                pegStatus: 1,
            },
            speedUpPubKey: FixedBytes::<32>::from([2u8; 32]),
            rskDestinationAddress: AlloyAddress::from([3u8; 20]),
            rbtcAmount: U256::from(1),
            utxoScriptPubKey: Bytes::from(vec![4u8]),
        }
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

    fn create_default_flow_context(
        flow_id: FlowId,
        step: Steps,
        op_role: Option<ParticipantRole>,
    ) -> FlowContext {
        // Default accept-pegin txid embedded in bitvmx_pegin_accepted; kept as
        // the zero hash so tests can assert against a known constant.
        let btc_tx_id = Txid::from_raw_hash(bitcoin::hashes::sha256d::Hash::all_zeros());
        FlowContext {
            flow_id,
            request_pegin_btc_tx_id: test_txid([1u8; 32]),
            step,
            // Most tests construct flows at steps where bitvmx_protocol_id
            // would already be set; supply a stable test value derived from
            // the flow's txid to mirror production.
            bitvmx_protocol_id: Some(accept_pegin_protocol_id(Uuid::nil(), 0)),
            request_pegin_btc_tx_status: None,
            request_pegin_spv_proof: None,
            pegin_requested: None,
            my_p2p_address: None,
            committee_output: None,
            bitvmx_pegin_accepted: Some(default_pegin_accepted_message(btc_tx_id)),
            operator_take_txid: None,
            operator_won_txid: None,
            accept_pegin_spv_proof: None,
            accept_pegin_tx_status: None,
            pegin_accepted: None,
            op_role,
        }
    }

    fn create_test_flow_with_mock_contracts(
        my_address: CommonAddress,
        ctx: FlowContext,
    ) -> (MockPeginFlow, std::rc::Rc<crate::coordinator::tests::MockRskContractsGatewayApi>) {
        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);
        create_test_flow_with_custom_mock(my_address, ctx, mock_contracts)
    }

    fn create_test_flow_with_custom_mock(
        my_address: CommonAddress,
        ctx: FlowContext,
        mut mock_contracts: crate::coordinator::tests::MockRskContractsGatewayApi,
    ) -> (MockPeginFlow, std::rc::Rc<crate::coordinator::tests::MockRskContractsGatewayApi>) {
        mock_contracts.expect_my_address().returning(move || my_address);
        let mock_contracts = std::rc::Rc::new(mock_contracts);

        let mock_broker = std::rc::Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let mock_store = std::rc::Rc::new(MockCoordinatorStoreApi::new());
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");

        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };

        let flow = PeginFlow::from_saved_state(
            mock_contracts.clone(),
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        (flow, mock_contracts)
    }

    fn create_test_flow_with_role(
        my_address: CommonAddress,
        step: Steps,
        op_role: Option<ParticipantRole>,
    ) -> (MockPeginFlow, std::rc::Rc<crate::coordinator::tests::MockRskContractsGatewayApi>) {
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let ctx = create_default_flow_context(flow_id, step, op_role);
        create_test_flow_with_mock_contracts(my_address, ctx)
    }

    fn create_store_backed_flow(
        ctx: FlowContext,
        mut mock_contracts: crate::coordinator::tests::MockRskContractsGatewayApi,
        store: std::rc::Rc<CoordinatorStore>,
    ) -> StoreBackedPeginFlow {
        mock_contracts.expect_my_address().returning(|| test_address([1u8; 20]));
        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        PeginFlow::from_saved_state(
            mock_contracts,
            RuntimeSync::new().expect("Failed to create runtime sync"),
            mock_broker,
            state,
            store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        )
    }

    #[test]
    fn request_pegin_retry_state_is_persisted_before_missing_native_bridge_error() {
        let store_path = TestStorePath::new();
        let store = std::rc::Rc::new(store_path.open());
        let spv_proof = test_spv_proof();
        let flow_id = flow_id_from_request_pegin_txid(spv_proof.tx.compute_txid());
        let ctx = create_default_flow_context(flow_id, Steps::RequestPeginSpvProof, None);
        let initial_state =
            State { flow_id, log_id: String::new(), ctx: ctx.clone(), created_at: None };
        store
            .save_flow(&StoreKey::PeginFlow(flow_id.value()), initial_state)
            .expect("initial flow state should persist");

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_request_pegin().times(1).returning(|_| {
            Err(DomainErrors::MissingConfirmationsOnNativeBridge("not enough blocks".to_string()))
        });
        let mut flow = create_store_backed_flow(ctx, mock_contracts, std::rc::Rc::clone(&store));

        let result = flow.complete_step(&StepData::RequestPeginSpvProof(spv_proof));
        assert!(result.is_err(), "missing native bridge confirmations should surface");

        let saved = store
            .load_flow::<State>(&StoreKey::PeginFlow(flow_id.value()))
            .expect("saved pegin flow should load")
            .expect("saved pegin flow should exist");
        assert_eq!(saved.ctx.step, Steps::RequestPeginSpvProof);
        assert!(
            saved.ctx.request_pegin_spv_proof.is_some(),
            "retry needs request-pegin SPV proof after restart"
        );
    }

    #[test]
    fn accept_pegin_retry_state_is_persisted_before_missing_native_bridge_error() {
        let store_path = TestStorePath::new();
        let store = std::rc::Rc::new(store_path.open());
        let spv_proof = test_spv_proof();
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let ctx = create_default_flow_context(flow_id, Steps::RequestAcceptPeginSpvProof, None);
        let initial_state =
            State { flow_id, log_id: String::new(), ctx: ctx.clone(), created_at: None };
        store
            .save_flow(&StoreKey::PeginFlow(flow_id.value()), initial_state)
            .expect("initial flow state should persist");

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_accept_pegin().times(1).returning(|_| {
            Err(DomainErrors::MissingConfirmationsOnNativeBridge("not enough blocks".to_string()))
        });
        let mut flow = create_store_backed_flow(ctx, mock_contracts, std::rc::Rc::clone(&store));

        let result = flow.complete_step(&StepData::AcceptPeginSpvProof(spv_proof));
        assert!(result.is_err(), "missing native bridge confirmations should surface");

        let saved = store
            .load_flow::<State>(&StoreKey::PeginFlow(flow_id.value()))
            .expect("saved pegin flow should load")
            .expect("saved pegin flow should exist");
        assert_eq!(saved.ctx.step, Steps::AcceptPegin);
        assert!(
            saved.ctx.accept_pegin_spv_proof.is_some(),
            "retry needs accept-pegin SPV proof after restart"
        );
    }

    fn create_committee_output_with_member(
        member_address: CommonAddress,
        role: u8,
    ) -> GetCommitteeOutput {
        use alloy_primitives::Address as AlloyAddress;
        use union_contracts::bindings::committee_registry::CommitteeRegistry::CommitteeMember;
        GetCommitteeOutput {
            committee: Committee {
                members: vec![CommitteeMember {
                    memberAddress: AlloyAddress::from_slice(
                        member_address.value().as_fixed_bytes(),
                    ),
                    role,
                }],
                leaderAddress: AlloyAddress::from_slice(&[0u8; 20]),
                operatorTakeIndex: U256::from(0),
                createdAt: Uint::default(),
                missingData: 0,
                missingCommunicationData: 0,
                isPending: false,
                streamId: 0,
                fundingUTXOs: vec![],
                aggregatedKey: vec![].into(),
            },
        }
    }

    #[test]
    fn test_add_operator_take_hash_prover_calls_contract() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let btc_tx_id = test_txid([0u8; 32]);

        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::AddOperatorTakeHash,
            Some(ParticipantRole::Prover),
        );
        ctx.bitvmx_pegin_accepted = Some(default_pegin_accepted_message(btc_tx_id));
        ctx.operator_take_txid = Some(test_txid([1u8; 32]));
        ctx.operator_won_txid = Some(test_txid([2u8; 32]));

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);

        let expected_input = transaction_dispatcher::types::AddOperatorTakeTxHashInput {
            accept_pegin_tx_hash: btc_tx_id,
            take_tx_hash: test_txid([1u8; 32]),
            won_tx_hash: test_txid([2u8; 32]),
        };

        mock_contracts
            .expect_add_operator_take_tx_hash()
            .with(eq(expected_input))
            .times(1)
            .returning(|_| {
                Ok(transaction_dispatcher::types::AddOperatorTakeTxHashOutput {
                    transaction_hash: "test_hash".to_string(),
                })
            });

        let (flow, _) = create_test_flow_with_custom_mock(my_address, ctx, mock_contracts);
        let result = flow.add_operator_take_hash();
        assert!(result.is_ok());
    }

    #[test]
    fn test_request_operator_take_transaction_info_prover_stays_in_request_step() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let btc_tx_id = test_txid([0u8; 32]);

        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::RequestOperatorTakeTransactionInfo,
            Some(ParticipantRole::Prover),
        );
        ctx.bitvmx_pegin_accepted = Some(default_pegin_accepted_message(btc_tx_id));
        ctx.committee_output = Some(create_committee_output_with_member(my_address, ROLE_PROVER));

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);

        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(move |msg| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::GetTransactionInfoByName(id, name)
                        if *id == accept_pegin_protocol_id(Uuid::nil(), 0).value() && name == "OPERATOR_TAKE_TX_0"
                )
            })
            .times(1)
            .returning(|_| Ok(true));

        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(mock_broker);
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.start_step(Steps::RequestOperatorTakeTransactionInfo);
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::RequestOperatorTakeTransactionInfo);
    }

    #[test]
    fn test_operator_take_transaction_info_moves_to_won_step_and_caches_txid() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let txid = test_txid([3u8; 32]);

        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::RequestOperatorTakeTransactionInfo,
            Some(ParticipantRole::Prover),
        );
        ctx.committee_output = Some(create_committee_output_with_member(my_address, ROLE_PROVER));

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);

        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(move |msg| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::GetTransactionInfoByName(id, name)
                        if *id == accept_pegin_protocol_id(Uuid::nil(), 0).value() && name == "OPERATOR_WON_TX_0"
                )
            })
            .times(1)
            .returning(|_| Ok(true));

        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(mock_broker);
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.complete_step(&StepData::TransactionInfo {
            tx_name: flow.operator_take_transaction_name().unwrap(),
            txid,
        });
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::RequestOperatorWonTransactionInfo);
        assert_eq!(flow.get_state().ctx.operator_take_txid, Some(txid));
    }

    #[test]
    fn test_request_operator_won_transaction_info_prover_stays_in_request_step() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let btc_tx_id = test_txid([0u8; 32]);

        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::RequestOperatorWonTransactionInfo,
            Some(ParticipantRole::Prover),
        );
        ctx.bitvmx_pegin_accepted = Some(default_pegin_accepted_message(btc_tx_id));
        ctx.committee_output = Some(create_committee_output_with_member(my_address, ROLE_PROVER));

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);

        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(move |msg| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::GetTransactionInfoByName(id, name)
                        if *id == accept_pegin_protocol_id(Uuid::nil(), 0).value() && name == "OPERATOR_WON_TX_0"
                )
            })
            .times(1)
            .returning(|_| Ok(true));

        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(mock_broker);
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.start_step(Steps::RequestOperatorWonTransactionInfo);
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::RequestOperatorWonTransactionInfo);
    }

    #[test]
    fn test_operator_won_transaction_info_moves_to_add_step_and_caches_txid() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let txid = test_txid([4u8; 32]);
        let take_txid = test_txid([3u8; 32]);

        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::RequestOperatorWonTransactionInfo,
            Some(ParticipantRole::Prover),
        );
        ctx.committee_output = Some(create_committee_output_with_member(my_address, ROLE_PROVER));
        ctx.operator_take_txid = Some(take_txid);

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);
        let expected_input = transaction_dispatcher::types::AddOperatorTakeTxHashInput {
            accept_pegin_tx_hash: test_txid([0u8; 32]),
            take_tx_hash: take_txid,
            won_tx_hash: txid,
        };
        mock_contracts
            .expect_add_operator_take_tx_hash()
            .with(eq(expected_input))
            .times(1)
            .returning(|_| {
                Ok(transaction_dispatcher::types::AddOperatorTakeTxHashOutput {
                    transaction_hash: "test_hash".to_string(),
                })
            });

        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.complete_step(&StepData::TransactionInfo {
            tx_name: flow.operator_won_transaction_name().unwrap(),
            txid,
        });
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::WaitAllOperatorTakeTxidsAdded);
        assert_eq!(flow.get_state().ctx.operator_won_txid, Some(txid));
    }

    #[test]
    fn test_prepare_pegin_setup_verifier_skips_operator_info_step() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let btc_tx_id = test_txid([0u8; 32]);

        let ctx = create_default_flow_context(
            flow_id,
            Steps::PreparePeginSetup,
            Some(ParticipantRole::Verifier),
        );

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);

        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.complete_step(&StepData::BitvmxPeginAccepted(
            default_pegin_accepted_message(btc_tx_id),
        ));
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::WaitAllOperatorTakeTxidsAdded);
    }

    #[test]
    fn test_wait_all_operator_take_txids_added_has_no_entry_action() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let ctx = create_default_flow_context(
            flow_id,
            Steps::WaitAllOperatorTakeTxidsAdded,
            Some(ParticipantRole::Verifier),
        );

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);
        let mock_contracts = std::rc::Rc::new(mock_contracts);
        let mock_broker = std::rc::Rc::new(MockBrokerClientApi::<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >::new());
        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);
        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.start_step(Steps::WaitAllOperatorTakeTxidsAdded);
        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::WaitAllOperatorTakeTxidsAdded);
    }

    #[test]
    fn test_accept_pegin_signatures_ready_dispatches_transaction() {
        let my_address = test_address([1u8; 20]);
        let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
        let accept_pegin_txid = test_txid([8u8; 32]);
        let mut ctx = create_default_flow_context(
            flow_id,
            Steps::WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint,
            Some(ParticipantRole::Verifier),
        );
        ctx.bitvmx_pegin_accepted = Some(default_pegin_accepted_message(accept_pegin_txid));

        let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().returning(move || my_address);
        let mock_contracts = std::rc::Rc::new(mock_contracts);

        let mut mock_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        mock_broker
            .expect_send()
            .withf(move |msg| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::DispatchTransactionName(id, name)
                        if *id == accept_pegin_protocol_id(Uuid::nil(), 0).value() && name == "ACCEPT_PEGIN_TX"
                )
            })
            .times(1)
            .returning(|_| Ok(true));
        let mock_broker = std::rc::Rc::new(mock_broker);

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
        let mock_store = std::rc::Rc::new(mock_store);

        let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
        let state = State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
        let mut flow = PeginFlow::from_saved_state(
            mock_contracts,
            rt_sync,
            mock_broker,
            state,
            mock_store,
            std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.complete_step(&StepData::AcceptPeginSignaturesReady);

        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::ConfirmAcceptPeginTransaction);
    }

    #[test]
    fn test_pegin_accepted_completes_from_post_signature_steps() {
        let my_address = test_address([1u8; 20]);
        let accept_pegin_txid = test_txid([9u8; 32]);

        for step in [
            Steps::DispatchAcceptPeginTransactionAllConvergeCheckpoint,
            Steps::ConfirmAcceptPeginTransaction,
            Steps::RequestAcceptPeginSpvProof,
            Steps::AcceptPegin,
        ] {
            let flow_id = flow_id_from_request_pegin_txid(test_txid([1u8; 32]));
            let mut ctx =
                create_default_flow_context(flow_id, step, Some(ParticipantRole::Verifier));
            ctx.bitvmx_pegin_accepted = Some(default_pegin_accepted_message(accept_pegin_txid));

            let mut mock_contracts = crate::coordinator::tests::MockRskContractsGatewayApi::new();
            mock_contracts.expect_my_address().returning(move || my_address);
            let mock_contracts = std::rc::Rc::new(mock_contracts);

            let mut mock_broker =
                MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
            mock_broker.expect_send().times(1).returning(|_| Ok(true));
            let mock_broker = std::rc::Rc::new(mock_broker);

            let mut mock_store = MockCoordinatorStoreApi::new();
            mock_store.expect_save_flow::<State>().times(1).returning(|_, _| Ok(()));
            let mock_store = std::rc::Rc::new(mock_store);

            let rt_sync = RuntimeSync::new().expect("Failed to create runtime sync");
            let state =
                State { flow_id: ctx.flow_id, log_id: String::new(), ctx, created_at: None };
            let mut flow = PeginFlow::from_saved_state(
                mock_contracts,
                rt_sync,
                mock_broker,
                state,
                mock_store,
                std::rc::Rc::new(crate::flows::common::Signaling::new("/tmp", "disabled")),
                NativeBridgeVerifier::Dummy,
            );

            let pegin_accepted = default_pegin_accepted_event(accept_pegin_txid);
            let result = flow.complete_step(&StepData::PeginAccepted(pegin_accepted.clone()));

            assert!(result.is_ok());
            assert_eq!(flow.current_step(), Steps::Done);
            assert_eq!(flow.get_state().ctx.pegin_accepted, Some(pegin_accepted));
        }
    }

    #[test]
    fn test_calc_op_role_prover() {
        let my_address = test_address([1u8; 20]);
        let mut flow_state =
            create_test_flow_with_role(my_address, Steps::AddOperatorTakeHash, None).0;
        let committee_output = create_committee_output_with_member(my_address, ROLE_PROVER);
        flow_state.get_state_mut().ctx.committee_output = Some(committee_output);

        let result = flow_state.calc_op_role();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ParticipantRole::Prover);
    }

    #[test]
    fn test_calc_op_role_verifier() {
        let my_address = test_address([1u8; 20]);
        let mut flow_state =
            create_test_flow_with_role(my_address, Steps::AddOperatorTakeHash, None).0;
        let committee_output = create_committee_output_with_member(my_address, ROLE_VERIFIER);
        flow_state.get_state_mut().ctx.committee_output = Some(committee_output);

        let result = flow_state.calc_op_role();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ParticipantRole::Verifier);
    }
}
