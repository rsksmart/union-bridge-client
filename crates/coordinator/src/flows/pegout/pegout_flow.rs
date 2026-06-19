use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail, ensure};
use bitcoin::{PublicKey, Txid};
use common_bitvmx::bitvmx_types::{
    BitVmxProtocolId, BtcTxSPVProof, CommsAddress, IncomingBitVMXApiMessages, PegOutAccepted,
    PegOutRequest, PubKeyHash, TransactionStatus, VariableTypes, build_communication_data,
    user_take_protocol_id,
};
use common_broker::broker::BitVmxBrokerClientApi;
use common_core::types::{CommitteeId, TxHash};
use common_runtime::runtime_sync::RuntimeSync;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, trace, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{
    GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    P2PAddressParser, RegisterPegoutInput, TriggerOperatorTakeInput,
};
use union_contracts::bindings::pegout_manager::PegoutManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::flows::common::{COMM_KEY_INDEX, FlowId, Signaling};
use crate::flows::pegout::pegout_processor::PEGOUT_ACCEPTED_NAME;

/// Derive the pegout flow id from the `PegoutRequested` Rootstock tx hash.
#[must_use]
pub(crate) fn flow_id_from_pegout_requested_tx_hash(pegout_requested_tx_hash: TxHash) -> FlowId {
    FlowId::from_tx("pegout_flow", pegout_requested_tx_hash.value().as_bytes())
}

use crate::store::{CoordinatorStoreApi, StoreKey};
use crate::types::PegoutRegisteredEvent;

pub(crate) const PROGRAM_TYPE_USER_TAKE: &str = "take";
pub(crate) const USER_TAKE_TX: &str = "USER_TAKE_TX";
const PEGOUT_COMPLETED_VAR_NAME: &str = "PEG_OUT_COMPLETED";

/// Steps for the pegout state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum Steps {
    #[default]
    // Wait for the confirmed RSK PegoutRequested event.
    WaitPegoutRequested,
    // Authoritative checkpoint: entered after confirmed PegoutRequested establishes
    // the canonical user request.
    GetCommInfoAuthoritativeCheckpoint,
    PrepareUserTakeSetup,
    // Wait for the user-take signature flow.
    WaitUserTakeSignaturesReady,
    // All-converge checkpoint: entered after the user-take signing subflow completes.
    // Dispatch the fully signed user-take Bitcoin transaction.
    DispatchUserTakeTransactionAllConvergeCheckpoint,
    TriggerOperatorTake, // Triggered when timeout expires without signature completion
    ConfirmUserTakeTransaction,
    RequestUserTakeSpvProof,
    RegisterPegout,
    // Terminal state after confirmed RSK PegoutRegistered establishes the registerPegout result.
    Done,
    Failed,
}

impl Steps {
    fn allows_fast_forward_to_pegout_registered(self) -> bool {
        matches!(
            self,
            Steps::DispatchUserTakeTransactionAllConvergeCheckpoint
                | Steps::ConfirmUserTakeTransaction
                | Steps::RequestUserTakeSpvProof
                | Steps::RegisterPegout
        )
    }
}

/// Data passed between steps in the pegout flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StepData {
    PegoutRequested,
    CommInfo(CommsAddress),
    PegoutAccepted(PegOutAccepted),
    UserTakeSignaturesReady,
    UserTakeTransactionDispatched,
    TriggerOperatorTakeTimeout, // Timeout expired, trigger operator take
    TransactionConfirmed(TransactionStatus),
    SpvProof(BtcTxSPVProof),
    /// Retry register pegout without state transition data (for Native Bridge confirmation retries)
    RetryRegisterPegout,
    PegoutRegistered(PegoutRegisteredEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlowContext {
    pub pegout_requested: PegoutRequested,
    /// Canonical on-chain identifier (the `PegoutRequested` Rootstock tx
    /// hash). Set at flow creation from the wrapper `PegoutRequestedEvent`.
    pub pegout_requested_tx_hash: TxHash,
    /// `BitVMX` program id for the user-take program. Derived from
    /// `(committee_id, slot_index)` from `PegoutRequested` (which is available
    /// at flow creation, so this is non-optional). Used wherever `BitVMX`
    /// expects a program id — `Setup`, `SetVar`, `DispatchTransactionName`,
    /// `GetTransaction` — and matches `get_user_take_pid` on the `BitVMX` side.
    pub bitvmx_protocol_id: BitVmxProtocolId,
    pub my_p2p_address: Option<CommsAddress>,
    pub committee_output: Option<GetCommitteeOutput>,
    pub peg_out_accepted: Option<PegOutAccepted>,
    pub spv_proof: Option<BtcTxSPVProof>,
    pub pegout_registered: Option<PegoutRegistered>,
    pub pegout_registered_tx: Option<String>,
    pub transaction_status: Option<TransactionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct State {
    pub flow_id: FlowId,
    /// Pre-formatted display id for log lines. Not persisted — re-computed
    /// at construction and on `from_saved_state` via `build_log_id`.
    #[serde(skip)]
    pub log_id: String,
    pub step: Steps,
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
        format!("{} ({})", self.flow_id, self.ctx.pegout_requested_tx_hash)
    }
}

pub(crate) struct PegoutFlow<CG, BC, S>
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

impl<CG, BC, S> PegoutFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        pegout_requested: &crate::types::PegoutRequestedEvent,
        store: Rc<S>,
        signaling: Rc<Signaling>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Result<Self> {
        let flow_id = flow_id_from_pegout_requested_tx_hash(pegout_requested.tx_hash);
        let committee_uuid =
            Uuid::from_u128(u128::try_from(pegout_requested.inner.committeeId).map_err(|_| {
                anyhow!("committeeId {} too large for u128", pegout_requested.inner.committeeId)
            })?);
        let slot_index = usize::try_from(pegout_requested.inner.slotId)
            .map_err(|_| anyhow!("slotId {} too large for usize", pegout_requested.inner.slotId))?;
        let bitvmx_protocol_id = user_take_protocol_id(committee_uuid, slot_index);
        let mut state = State {
            flow_id,
            log_id: String::new(),
            step: Steps::WaitPegoutRequested,
            ctx: FlowContext {
                pegout_requested: pegout_requested.inner.clone(),
                pegout_requested_tx_hash: pegout_requested.tx_hash,
                bitvmx_protocol_id,
                my_p2p_address: None,
                committee_output: None,
                peg_out_accepted: None,
                pegout_registered: None,
                pegout_registered_tx: None,
                spv_proof: None,
                transaction_status: None,
            },
            created_at: Some(chrono::Utc::now()),
        };
        state.log_id = state.build_log_id();
        info!("PegoutFlow created");
        Ok(Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state,
            store,
            signaling,
            native_bridge_verifier,
        })
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
        debug!("Persisting state for step: {:?}", self.state.step);
        self.store.save_flow(&StoreKey::PegoutFlow(self.state.flow_id.value()), self.state.clone())
    }

    pub(crate) fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.step;
        self.state.step = next_step;

        debug!("{} -> {}", format_step(previous_step), format_step(next_step));

        if next_step == Steps::RegisterPegout {
            self.persist_state()?;
        }

        match next_step {
            Steps::WaitPegoutRequested => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::GetCommInfoAuthoritativeCheckpoint => {
                self.request_bitvmx_comm_info()?;
            }
            //This step will prepare the user take setup by sending the setVar and setup to bitvmx in a single step to make bitvmx complete the pegout setup step.
            Steps::PrepareUserTakeSetup => {
                self.communicate_pegout_requested_to_bitvmx()?;
            }
            Steps::WaitUserTakeSignaturesReady => {
                info!("Waiting for signatures to be ready to dispatch transaction");
            }
            Steps::DispatchUserTakeTransactionAllConvergeCheckpoint => {
                self.dispatch_transaction()?;
                self.complete_step(&StepData::UserTakeTransactionDispatched)?;
            }
            Steps::TriggerOperatorTake => {
                info!("Triggering operator take due to timeout");
                let Err(err) = self.trigger_operator_take() else {
                    info!("TriggerOperatorTake completed");
                    return Ok(());
                };
                warn!("Failed to trigger operator take: {err}. Continuing flow.");
                info!("TriggerOperatorTake skipped");
            }
            Steps::ConfirmUserTakeTransaction => {
                info!(
                    "Waiting for UserTake Bitcoin confirmations: user_take_tx_id={:?}",
                    self.get_user_take_txid()
                );
            }
            Steps::RequestUserTakeSpvProof => {
                info!("Requesting SPV proof: user_take_tx_id={:?}", self.get_user_take_txid());
                self.request_spv_proof()?;
            }
            Steps::RegisterPegout => {
                info!("Registering pegout");
                let spv_proof =
                    self.state.ctx.spv_proof.as_ref().ok_or_else(|| {
                        anyhow!("SPV proof not available for pegout registration")
                    })?;
                self.register_pegout(spv_proof)?;
            }
            Steps::Done => {
                if self.state.ctx.pegout_registered_tx.is_some() {
                    self.write_completion_marker()?;
                    metrics::counter!("union_flows_completed_total", "type" => "pegout")
                        .increment(1);
                    metrics::counter!("union_pegout_amount_sats_total")
                        .increment(self.state.ctx.pegout_requested.amount);
                    info!("Done");
                }
            }
            Steps::Failed => {
                info!("Failed");
            }
        }

        self.persist_state()?;

        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub(crate) fn complete_step(&mut self, data: &StepData) -> Result<()> {
        let current_step: Steps = self.state.step;

        debug!("Completing step {} with data: {:?}", format_step(current_step), data);

        // Process data and determine next state
        let next_step = self.process_step_data(current_step, data)?;

        // Transition to the next state
        self.start_step(next_step)?;

        Ok(())
    }

    fn process_step_data(&mut self, current_step: Steps, data: &StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::WaitPegoutRequested, StepData::PegoutRequested) => {
                Ok(Steps::GetCommInfoAuthoritativeCheckpoint)
            }
            (Steps::GetCommInfoAuthoritativeCheckpoint, StepData::CommInfo(comm_info)) => {
                self.state.ctx.my_p2p_address = Some(comm_info.clone());
                Ok(Steps::PrepareUserTakeSetup)
            }
            (Steps::PrepareUserTakeSetup, StepData::PegoutAccepted(accepted)) => {
                self.state.ctx.peg_out_accepted = Some(accepted.clone());
                Ok(Steps::WaitUserTakeSignaturesReady)
            }
            (Steps::WaitUserTakeSignaturesReady, StepData::UserTakeSignaturesReady) => {
                Ok(Steps::DispatchUserTakeTransactionAllConvergeCheckpoint)
            }
            (
                Steps::DispatchUserTakeTransactionAllConvergeCheckpoint,
                StepData::UserTakeTransactionDispatched,
            ) => Ok(Steps::ConfirmUserTakeTransaction),
            (Steps::WaitUserTakeSignaturesReady, StepData::TriggerOperatorTakeTimeout) => {
                // Timeout expired, transition to TriggerOperatorTake step
                info!("Timeout expired, transitioning to TriggerOperatorTake");
                Ok(Steps::TriggerOperatorTake)
            }
            (Steps::TriggerOperatorTake, StepData::TriggerOperatorTakeTimeout) => {
                // After TriggerOperatorTake step completes, finish the flow
                info!("TriggerOperatorTake step completed, completing flow");
                Ok(Steps::Done)
            }
            (Steps::ConfirmUserTakeTransaction, StepData::TransactionConfirmed(tx_status)) => {
                info!("Transaction confirmed: tx_id={:?}", tx_status.tx_id);
                trace!("Transaction status data: {tx_status:?}");
                let expected_tx_id = self
                    .get_user_take_txid()
                    .ok_or_else(|| anyhow!("Expected user take txid not found"))?;
                ensure!(
                    tx_status.tx_id == expected_tx_id,
                    "Transaction status txId mismatch: got {:?}, expected {:?}",
                    tx_status.tx_id,
                    expected_tx_id
                );
                self.state.ctx.transaction_status = Some(tx_status.clone());
                Ok(Steps::RequestUserTakeSpvProof)
            }
            (Steps::RequestUserTakeSpvProof, StepData::SpvProof(spv_proof)) => {
                info!("Received SPV proof");
                trace!("SPV Proof data: {spv_proof:?}");
                self.state.ctx.spv_proof = Some(spv_proof.clone());
                Ok(Steps::RegisterPegout)
            }
            (Steps::RegisterPegout, StepData::RetryRegisterPegout) => {
                info!("Retrying register pegout");
                Ok(Steps::RegisterPegout)
            }
            (step, StepData::PegoutRegistered(pegout_registered))
                if step.allows_fast_forward_to_pegout_registered() =>
            {
                self.complete_with_confirmed_pegout_registration(pegout_registered)
            }
            _ => Err(anyhow::anyhow!(
                "Invalid state transition: {current_step:?} with data {data:?}"
            )),
        }
    }

    fn complete_with_confirmed_pegout_registration(
        &mut self,
        pegout_registered: &PegoutRegisteredEvent,
    ) -> Result<Steps> {
        // A confirmed PegoutRegistered is authoritative only after the flow has passed
        // the shared PegoutAccepted checkpoint. From that point on, every operator
        // knows the canonical user_take_txid, so lagging flows can safely converge
        // directly to Done without replaying the remaining local BitVMX/contract work.
        let expected_tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take txid not found"))?;
        let registered_tx_id =
            common_core::types::TxIdParser::fb_32_to_txid(pegout_registered.inner.txid);

        ensure!(
            registered_tx_id == expected_tx_id,
            "PegoutRegistered txid mismatch: got {registered_tx_id:?}, expected {expected_tx_id:?}"
        );

        info!("Pegout registered successfully");
        trace!("PegoutRegistered data: {:?}", pegout_registered.inner);

        self.state.ctx.pegout_registered = Some(pegout_registered.inner.clone());
        self.state.ctx.pegout_registered_tx = Some(pegout_registered.tx_hash.to_string());
        self.send_pegout_completed_to_bitvmx(&pegout_registered.inner)?;

        Ok(Steps::Done)
    }

    //This step will send the setVar and setup to bitvmx in a single step to make bitvmx complete the pegout setup step.
    fn communicate_pegout_requested_to_bitvmx(&mut self) -> Result<()> {
        info!("Communicating pegout requested to bitvmx");
        let committee_id: CommitteeId = self.state.ctx.pegout_requested.committeeId.try_into()?;

        self.send_pegout_requested_to_bitvmx(&committee_id)?;
        self.send_setup_to_bitvmx(&committee_id)?;
        Ok(())
    }

    fn get_committee_output(&mut self, committee_id: CommitteeId) -> Result<GetCommitteeOutput> {
        let committee_response = self.rt_sync.run(async {
            self.contracts.get_committee(GetCommitteeInput { committee_id }).await
        })?;
        Ok(committee_response)
    }
    fn send_setup_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!("Sending setup to bitvmx");
        let committee_pubkey_hashes = self.get_committee_pubkey_hashes(
            self.state
                .ctx
                .committee_output
                .clone()
                .ok_or_else(|| anyhow!("Committee output not available for setup"))?,
        )?;

        let committee_addresses = self.get_committee_member_address(committee_id)?;
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
            self.state.ctx.bitvmx_protocol_id.value(),
            PROGRAM_TYPE_USER_TAKE.to_string(),
            comms_addresses,
            0,
        );
        self.send_bitvmx_msg(msg)
    }

    fn get_committee_member_address(&mut self, committee_id: &CommitteeId) -> Result<Vec<String>> {
        let input = GetCommunicationDataInput {
            committee_id: committee_id.clone(),
            member_address: self.contracts.my_address().into(),
        };
        let member_comm_data = self
            .rt_sync
            .run(async { self.contracts.get_committee_communication_data(input).await })?;

        let committee_addresses = member_comm_data
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

    fn get_committee_pubkey_hashes(
        &mut self,
        committee_output: GetCommitteeOutput,
    ) -> Result<Vec<PubKeyHash>> {
        let mut pubkey_hashes = Vec::new();

        for member in committee_output.committee.members {
            // Get the member's public keys
            let keys_input = GetMemberPublicKeysInput { member_address: member.memberAddress };

            let keys_response = self
                .rt_sync
                .run(async { self.contracts.get_member_public_keys(keys_input).await })?;

            // Get the communication key (at index 2)
            let key_str = keys_response.public_keys.get(COMM_KEY_INDEX).context(format!(
                "Communication key not found for member {}",
                member.memberAddress
            ))?;

            debug!("Member {} pubkey_hash: {:?}", member.memberAddress, key_str);
            pubkey_hashes.push(key_str.clone());
        }

        Ok(pubkey_hashes)
    }

    fn send_pegout_requested_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!("Notifying pegout requested to bitvmx");
        let committee_output: GetCommitteeOutput =
            self.get_committee_output(committee_id.clone())?;
        self.state.ctx.committee_output = Some(committee_output.clone());
        let data_to_send: PegOutRequest = Self::pegout_requested_to_bitvmx_request(
            &self.state.ctx.pegout_requested,
            &committee_output,
        )?;

        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.ctx.bitvmx_protocol_id.value(),
            PegOutRequest::name().to_string(),
            VariableTypes::String(serde_json::to_string(&data_to_send)?),
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn send_pegout_completed_to_bitvmx(
        &mut self,
        pegout_registered: &PegoutRegistered,
    ) -> Result<()> {
        debug!("Notifying pegout completed to bitvmx");
        let data = serde_json::to_string(&pegout_registered)?;
        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.ctx.bitvmx_protocol_id.value(),
            PEGOUT_COMPLETED_VAR_NAME.to_string(),
            VariableTypes::String(data),
        );

        self.send_bitvmx_msg(msg)
    }

    fn write_completion_marker(&self) -> Result<()> {
        let payload = json!({
            "request_pegout_tx_hash": self.state.ctx.pegout_requested_tx_hash.to_string(),
            "committee_id": self.state.ctx.pegout_requested.committeeId.to_string(),
            "stream_id": self.state.ctx.pegout_requested.streamId,
            "packet_number": self.state.ctx.pegout_requested.packetNumber,
            "slot_id": self.state.ctx.pegout_requested.slotId,
            "amount": self.state.ctx.pegout_requested.amount.to_string(),
            "user_take_txid": self.state.ctx.peg_out_accepted.as_ref().map(|accepted| accepted.user_take_txid.to_string()),
            "registered_txid": self.state.ctx.pegout_registered_tx,
        });

        self.signaling.signal_done("pegout", self.state.ctx.bitvmx_protocol_id.value(), &payload)
    }

    fn pegout_requested_to_bitvmx_request(
        event: &PegoutRequested,
        committee_output: &GetCommitteeOutput,
    ) -> Result<PegOutRequest> {
        debug!("Preparing PegOutRequest for BitVMX from PegoutRequested event: {event:?}");

        let committee_id: Uuid = Uuid::from_u128(event.committeeId.try_into()?);

        let user_pubkey = if event.userPubKey.len() == 33 {
            debug!("Attempting to parse as compressed public key (33 bytes)");
            PublicKey::from_slice(event.userPubKey.as_ref())
                .context("Failed to parse user public key as compressed public key")?
        } else {
            bail!("Invalid user public key length: {}, expected 33", event.userPubKey.len());
        };

        let take_aggregated_key = Self::build_take_aggregated_key(committee_output)?;

        let pegout_sighash: Vec<u8> = event.pegoutSignatureData.signatureHash.to_vec();
        let pegout_id: Vec<u8> = event.pegoutSignatureData.txid.to_vec();

        let slot_index =
            usize::try_from(event.slotId).map_err(|_| anyhow!("slotId too large for usize"))?;

        Ok(PegOutRequest {
            committee_id,
            slot_index,
            amount: event.amount,
            pegout_id,
            user_pubkey,
            pegout_sighash,
            take_aggregated_key,
        })
    }

    fn build_take_aggregated_key(committee_response: &GetCommitteeOutput) -> Result<PublicKey> {
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        info!("Requesting bitvmx comm info");
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    fn dispatch_transaction(&self) -> Result<()> {
        info!("Dispatching {}", USER_TAKE_TX);
        let msg = IncomingBitVMXApiMessages::DispatchTransactionName(
            self.state.ctx.bitvmx_protocol_id.value(),
            USER_TAKE_TX.to_string(),
        );
        self.send_bitvmx_msg(msg)?;
        Ok(())
    }

    pub(crate) fn get_user_take_txid(&self) -> Option<Txid> {
        self.state.ctx.peg_out_accepted.as_ref().map(|accepted| accepted.user_take_txid)
    }

    /// Get the pegout txid from the `PegoutRequested` event
    pub(crate) fn get_pegout_txid(&self) -> String {
        hex::encode(self.state.ctx.pegout_requested.pegoutSignatureData.txid.as_slice())
    }

    fn register_pegout(&self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        let input = RegisterPegoutInput::from(spv_proof.clone());

        invoke_contract_safe(
            &self.rt_sync,
            "registerPegout",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.register_pegout(input).await },
        )
        .context("Failed to register pegout with provided SPV proof")?;

        info!("Pegout registration sent");
        Ok(())
    }

    pub(crate) fn request_transaction_status(&self) -> Result<()> {
        let tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        info!("Requesting transaction status: tx_id={:?}", tx_id);
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(
            self.state.ctx.bitvmx_protocol_id.value(),
            tx_id,
        ))?;
        Ok(())
    }

    pub(crate) fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))?;
        Ok(())
    }

    /// Re-pull the one-time `pegout_accepted` variable from `BitVMX`.
    ///
    /// The push that advances `PrepareUserTakeSetup` is delivered once and is not
    /// redelivered across a restart, so on recovery we query it explicitly. The
    /// reply re-enters the existing `Variable` handler. Only the query is re-sent
    /// — never the `Setup` that produced it, which `BitVMX` rejects for an
    /// already-set-up protocol.
    fn request_pegout_accepted_var(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetVar(
            self.bitvmx_protocol_id().value(),
            PEGOUT_ACCEPTED_NAME.to_string(),
        ))
    }

    /// On recovery, re-issue the outbound `BitVMX` query for a step that is
    /// blocked waiting on a one-time `BitVMX` push. `BitVMX` does not redeliver
    /// these pushes across a restart, so without this the flow stalls forever.
    /// Each step re-sends only an idempotent `Get*` query, not the mutating
    /// request that originally triggered the push.
    pub(crate) fn redrive_pending_bitvmx_request(&self) -> Result<()> {
        match self.current_step() {
            Steps::PrepareUserTakeSetup => self.request_pegout_accepted_var(),
            Steps::GetCommInfoAuthoritativeCheckpoint => self.request_bitvmx_comm_info(),
            Steps::RequestUserTakeSpvProof => self.request_spv_proof(),
            _ => Ok(()),
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self.state.step, Steps::Done | Steps::Failed)
    }

    pub(crate) fn mark_failed(&mut self, reason: &str) -> Result<()> {
        info!("Marking as failed: {reason}");
        self.start_step(Steps::Failed)
    }

    /// Get the flow id (a `Uuid` derived from `pegout_requested_tx_hash`).
    pub(crate) fn flow_id(&self) -> FlowId {
        self.state.flow_id
    }

    /// Pre-formatted display id for log lines.
    pub(crate) fn log_id(&self) -> &str {
        &self.state.log_id
    }

    /// `BitVMX` program id for the user-take program. Available from flow
    /// creation, since `(committee_id, slot_index)` are present in
    /// `PegoutRequested`.
    pub(crate) fn bitvmx_protocol_id(&self) -> BitVmxProtocolId {
        self.state.ctx.bitvmx_protocol_id
    }

    pub(crate) fn current_step(&self) -> Steps {
        self.state.step
    }

    pub(crate) fn get_flow_details(&self) -> crate::event_processor::FlowDetails {
        crate::event_processor::FlowDetails {
            kind: crate::types::FlowKind::Pegout,
            id: self.flow_id().to_string(),
            step: format!("{:?}", self.current_step()),
            created_at: self.state.created_at,
        }
    }

    #[cfg(test)]
    pub(crate) fn get_state(&self) -> &State {
        &self.state
    }

    pub(crate) fn pegout_requested(&self) -> &PegoutRequested {
        &self.state.ctx.pegout_requested
    }

    /// Trigger operator take when timeout expires
    fn trigger_operator_take(&self) -> Result<()> {
        let pegout_txid = self.get_pegout_txid();

        info!("Calling trigger_operator_take: pegout_txid={pegout_txid}");

        let input = TriggerOperatorTakeInput { pegout_txid };

        let output =
            match self.rt_sync.run(async { self.contracts.trigger_operator_take(input).await }) {
                Ok(output) => output,
                Err(domain_err) => {
                    anyhow::bail!("Failed to trigger operator take: {domain_err:?}");
                }
            };

        info!("trigger_operator_take called successfully: tx_hash={}", output.transaction_hash);

        Ok(())
    }
}

/// Helper function to format step names
fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::WaitPegoutRequested => "WaitPegoutRequested",
        Steps::GetCommInfoAuthoritativeCheckpoint => "GetCommInfoAuthoritativeCheckpoint",
        Steps::PrepareUserTakeSetup => "PrepareUserTakeSetup",
        Steps::WaitUserTakeSignaturesReady => "WaitUserTakeSignaturesReady",
        Steps::DispatchUserTakeTransactionAllConvergeCheckpoint => {
            "DispatchUserTakeTransactionAllConvergeCheckpoint"
        }
        Steps::TriggerOperatorTake => "TriggerOperatorTake",
        Steps::ConfirmUserTakeTransaction => "ValidateTransactionStatus",
        Steps::RequestUserTakeSpvProof => "RequestSpvProof",
        Steps::RegisterPegout => "RegisterPegout",
        Steps::Done => "Done",
        Steps::Failed => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::{env, fs};

    use alloy_primitives::{Bytes, FixedBytes, U256 as AlloyU256};
    use bitcoin::Txid;
    use common_bitvmx::bitvmx_types::{
        BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, PegOutAccepted,
        VariableTypes,
    };
    use common_broker::broker::MockBrokerClientApi;
    use common_core::types::{BlockHash, BlockNumber, TxHash, TxIdParser};
    use common_runtime::runtime_sync::RuntimeSync;
    use mockall::predicate::function;
    use musig2::PubNonce;
    use musig2::secp::MaybeScalar;
    use primitive_types::H256;
    use union_contracts::bindings::pegout_manager::PegoutManager::{
        BitcoinSignatureData, BtcTransaction, StreamPosition,
    };
    use uuid::Uuid;

    use super::*;
    use crate::RUNTIME_ENV_LOCAL_ANVIL;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::store::{CoordinatorStore, MockCoordinatorStoreApi, TestStorePath};

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;
    type TestPegoutFlow =
        PegoutFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>;
    type StoreBackedPegoutFlow =
        PegoutFlow<MockRskContractsGatewayApi, MockBitVmxBroker, CoordinatorStore>;

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("pegout-flow-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
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

    fn fake_pegout_requested() -> crate::types::PegoutRequestedEvent {
        crate::types::EventWithBlock {
            inner: PegoutRequested {
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
            },
            block_number: BlockNumber::from(10),
            block_hash: BlockHash::from(H256::from_low_u64_be(11)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(12)),
        }
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

    fn test_spv_proof() -> BtcTxSPVProof {
        BtcTxSPVProof {
            block_hash: "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            tx: bitcoin::Transaction {
                version: bitcoin::transaction::Version(2),
                lock_time: bitcoin::absolute::LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            merkle_branch_path: "0".to_string(),
            merkle_branch_hashes: vec![],
        }
    }

    fn fake_pegout_registered_event(user_take_txid: Txid) -> PegoutRegisteredEvent {
        crate::types::EventWithBlock {
            inner: PegoutRegistered {
                blockHash: FixedBytes::from([1u8; 32]),
                txid: common_core::types::TxIdParser::txid_to_fb_32(user_take_txid),
                acceptPeginTxid: FixedBytes::from([2u8; 32]),
                committeeId: 1,
                streamInfo: StreamPosition {
                    streamId: 0,
                    packetNumber: 0,
                    slotId: 0,
                    pegStatus: 0,
                },
            },
            block_number: BlockNumber::from(20),
            block_hash: BlockHash::from(H256::from_low_u64_be(21)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(22)),
        }
    }

    fn create_store_backed_flow(
        state: State,
        contracts: MockRskContractsGatewayApi,
        store: Rc<CoordinatorStore>,
    ) -> StoreBackedPegoutFlow {
        PegoutFlow::from_saved_state(
            Rc::new(contracts),
            RuntimeSync::new().expect("runtime"),
            Rc::new(MockBitVmxBroker::new()),
            state,
            store,
            Rc::new(Signaling::new("/tmp", "disabled")),
            NativeBridgeVerifier::Dummy,
        )
    }

    #[test]
    fn register_pegout_retry_state_is_persisted_before_missing_native_bridge_error() {
        let store_path = TestStorePath::new();
        let store = Rc::new(store_path.open());
        let flow_id =
            flow_id_from_pegout_requested_tx_hash(TxHash::from(H256::from_low_u64_be(99)));
        let state = State {
            flow_id,
            log_id: String::new(),
            step: Steps::RequestUserTakeSpvProof,
            ctx: FlowContext {
                pegout_requested: fake_pegout_requested().inner,
                pegout_requested_tx_hash: TxHash::from(H256::from_low_u64_be(99)),
                bitvmx_protocol_id: user_take_protocol_id(Uuid::nil(), 0),
                my_p2p_address: None,
                committee_output: None,
                peg_out_accepted: Some(fake_pegout_accepted(test_txid([3u8; 32]))),
                pegout_registered: None,
                pegout_registered_tx: None,
                spv_proof: None,
                transaction_status: None,
            },
            created_at: None,
        };
        store
            .save_flow(&StoreKey::PegoutFlow(flow_id.value()), state.clone())
            .expect("initial pegout flow state should persist");

        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_register_pegout().times(1).returning(|_| {
            Err(transaction_dispatcher::rsk_gateway::DomainErrors::MissingConfirmationsOnNativeBridge(
                "not enough blocks".to_string(),
            ))
        });
        let mut flow = create_store_backed_flow(state, contracts, Rc::clone(&store));

        let result = flow.complete_step(&StepData::SpvProof(test_spv_proof()));
        assert!(result.is_err(), "missing native bridge confirmations should surface");

        let saved = store
            .load_flow::<State>(&StoreKey::PegoutFlow(flow_id.value()))
            .expect("saved pegout flow should load")
            .expect("saved pegout flow should exist");
        assert_eq!(saved.step, Steps::RegisterPegout);
        assert!(
            saved.ctx.spv_proof.is_some(),
            "retry needs register-pegout SPV proof after restart"
        );
    }

    fn completion_marker_path(root: &TempDir, flow_id: Uuid) -> std::path::PathBuf {
        root.path()
            .join("union-bridge-flow-completion-markers")
            .join(format!("pegout-{flow_id}.json"))
    }

    fn build_flow(
        initial_step: Steps,
        signaling_root: &TempDir,
        expect_completion_msg: bool,
        expect_persist: bool,
    ) -> TestPegoutFlow {
        let contracts = MockRskContractsGatewayApi::new();

        let mut broker = MockBitVmxBroker::new();
        if expect_completion_msg {
            broker
                .expect_send()
                .times(1)
                .with(function(|msg: &IncomingBitVMXApiMessages| {
                    matches!(
                        msg,
                        IncomingBitVMXApiMessages::SetVar(_, name, VariableTypes::String(_))
                            if name == PEGOUT_COMPLETED_VAR_NAME
                    )
                }))
                .returning(|_| Ok(true));
        } else {
            broker.expect_send().times(0);
        }

        let mut store = MockCoordinatorStoreApi::new();
        store
            .expect_save_flow::<State>()
            .times(usize::from(expect_persist))
            .returning(|_, _| Ok(()));

        let user_take_txid = test_txid([3u8; 32]);
        let ctx = FlowContext {
            pegout_requested: fake_pegout_requested().inner,
            pegout_requested_tx_hash: TxHash::from(H256::from_low_u64_be(99)),
            bitvmx_protocol_id: user_take_protocol_id(Uuid::nil(), 0),
            my_p2p_address: None,
            committee_output: None,
            peg_out_accepted: Some(fake_pegout_accepted(user_take_txid)),
            spv_proof: (initial_step == Steps::RegisterPegout).then(|| BtcTxSPVProof {
                block_hash: "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
                tx: bitcoin::Transaction {
                    version: bitcoin::transaction::Version(2),
                    lock_time: bitcoin::absolute::LockTime::ZERO,
                    input: vec![],
                    output: vec![],
                },
                merkle_branch_path: "0".to_string(),
                merkle_branch_hashes: vec![],
            }),
            pegout_registered: None,
            pegout_registered_tx: None,
            transaction_status: None,
        };

        PegoutFlow::from_saved_state(
            Rc::new(contracts),
            RuntimeSync::new().expect("runtime"),
            Rc::new(broker),
            State {
                flow_id: flow_id_from_pegout_requested_tx_hash(TxHash::from(
                    H256::from_low_u64_be(99),
                )),
                log_id: String::new(),
                step: initial_step,
                ctx,
                created_at: None,
            },
            Rc::new(store),
            Rc::new(Signaling::new(signaling_root.path(), RUNTIME_ENV_LOCAL_ANVIL)),
            NativeBridgeVerifier::Dummy,
        )
    }

    #[test]
    fn confirmed_pegout_registered_terminalizes_from_register_pegout() {
        let tempdir = TempDir::new();
        let mut flow = build_flow(Steps::RegisterPegout, &tempdir, true, true);
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        flow.complete_step(&StepData::PegoutRegistered(event.clone())).expect("flow completes");

        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );

        let marker: serde_json::Value = serde_json::from_slice(
            &fs::read(completion_marker_path(&tempdir, flow.bitvmx_protocol_id().value()))
                .expect("marker exists"),
        )
        .expect("marker json");
        assert_eq!(
            marker["payload"]["registered_txid"].as_str(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn confirmed_pegout_registered_terminalizes_from_request_user_take_spv_proof() {
        let tempdir = TempDir::new();
        let mut flow = build_flow(Steps::RequestUserTakeSpvProof, &tempdir, true, true);
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        flow.complete_step(&StepData::PegoutRegistered(event.clone())).expect("flow completes");

        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn confirmed_pegout_registered_terminalizes_from_confirm_user_take_transaction() {
        let tempdir = TempDir::new();
        let mut flow = build_flow(Steps::ConfirmUserTakeTransaction, &tempdir, true, true);
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        flow.complete_step(&StepData::PegoutRegistered(event.clone())).expect("flow completes");

        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn confirmed_pegout_registered_terminalizes_from_dispatch_transaction() {
        let tempdir = TempDir::new();
        let mut flow = build_flow(
            Steps::DispatchUserTakeTransactionAllConvergeCheckpoint,
            &tempdir,
            true,
            true,
        );
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        flow.complete_step(&StepData::PegoutRegistered(event.clone())).expect("flow completes");

        assert_eq!(flow.current_step(), Steps::Done);
        assert_eq!(flow.get_state().ctx.pegout_registered.as_ref(), Some(&event.inner));
        assert_eq!(
            flow.get_state().ctx.pegout_registered_tx.as_deref(),
            Some(event.tx_hash.to_string().as_str())
        );
    }

    #[test]
    fn user_take_signatures_ready_dispatches_transaction() {
        let tempdir = TempDir::new();
        let contracts = MockRskContractsGatewayApi::new();
        let pegout_requested_tx_hash = TxHash::from(H256::from_low_u64_be(123));
        let flow_id = flow_id_from_pegout_requested_tx_hash(pegout_requested_tx_hash);
        let protocol_id = user_take_protocol_id(Uuid::nil(), 0);

        let mut broker = MockBitVmxBroker::new();
        broker
            .expect_send()
            .times(1)
            .with(function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::DispatchTransactionName(id, name)
                        if *id == protocol_id.value() && name == USER_TAKE_TX
                )
            }))
            .returning(|_| Ok(true));

        let mut store = MockCoordinatorStoreApi::new();
        store.expect_save_flow::<State>().times(2).returning(|_, _| Ok(()));

        let user_take_txid = test_txid([3u8; 32]);
        let ctx = FlowContext {
            pegout_requested: fake_pegout_requested().inner,
            pegout_requested_tx_hash,
            bitvmx_protocol_id: user_take_protocol_id(Uuid::nil(), 0),
            my_p2p_address: None,
            committee_output: None,
            peg_out_accepted: Some(fake_pegout_accepted(user_take_txid)),
            spv_proof: None,
            pegout_registered: None,
            pegout_registered_tx: None,
            transaction_status: None,
        };

        let mut flow = PegoutFlow::from_saved_state(
            Rc::new(contracts),
            RuntimeSync::new().expect("runtime"),
            Rc::new(broker),
            State {
                flow_id,
                log_id: String::new(),
                step: Steps::WaitUserTakeSignaturesReady,
                ctx,
                created_at: None,
            },
            Rc::new(store),
            Rc::new(Signaling::new(tempdir.path(), RUNTIME_ENV_LOCAL_ANVIL)),
            NativeBridgeVerifier::Dummy,
        );

        let result = flow.complete_step(&StepData::UserTakeSignaturesReady);

        assert!(result.is_ok());
        assert_eq!(flow.current_step(), Steps::ConfirmUserTakeTransaction);
    }

    #[test]
    fn confirmed_pegout_registered_is_rejected_before_shared_checkpoint() {
        let tempdir = TempDir::new();
        let mut flow = build_flow(Steps::WaitUserTakeSignaturesReady, &tempdir, false, false);
        let event = fake_pegout_registered_event(test_txid([3u8; 32]));

        let result = flow.complete_step(&StepData::PegoutRegistered(event));

        assert!(result.is_err());
        assert_eq!(flow.current_step(), Steps::WaitUserTakeSignaturesReady);
        assert!(flow.get_state().ctx.pegout_registered.is_none());
        assert!(flow.get_state().ctx.pegout_registered_tx.is_none());
    }
}
