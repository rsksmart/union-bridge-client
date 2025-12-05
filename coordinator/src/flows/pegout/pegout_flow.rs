use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail, ensure};
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, IncomingBitVMXApiMessages, P2PAddress, PeerId, PegOutAccepted, PegOutRequest,
    TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use common::types::CommitteeId;
use hex;
use log::{debug, info, trace, warn};
use serde::{Deserialize, Serialize};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{
    GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    P2PAddressParser, RegisterPegoutInput, RegisterPegoutOutput, TriggerOperatorTakeInput,
};
use union_contracts::bindings::pegout_manager::PegoutManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

use crate::flows::common::{COMM_KEY_INDEX, build_communication_data};
use crate::store::{CoordinatorStoreApi, StoreKey};

pub const PROGRAM_TYPE_USER_TAKE: &str = "take";
pub const USER_TAKE_TX: &str = "USER_TAKE_TX";
const PEGOUT_COMPLETED_VAR_NAME: &str = "PEG_OUT_COMPLETED";

/// Steps for the pegout state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Steps {
    #[default]
    PegoutRequested,
    GetCommInfo,
    PrepareUserTakeSetup,
    //The signature flow is being executed Between these two steps and outside the flow.
    DispatchTransaction,
    TriggerOperatorTake, // Triggered when timeout expires without signature completion
    ConfirmUserTakeTransaction,
    RequestUserTakeSpvProof,
    RegisterPegout,
    Done,
}

/// Data passed between steps in the pegout flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepData {
    PegoutRequested,
    CommInfo(P2PAddress),
    PegoutAccepted(PegOutAccepted),
    DispatchTransaction,
    TriggerOperatorTakeTimeout, // Timeout expired, trigger operator take
    TransactionConfirmed(TransactionStatus),
    SpvProof(BtcTxSPVProof),
    PegoutRegistered(PegoutRegistered),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowContext {
    pub pegout_requested: PegoutRequested,
    pub my_p2p_address: Option<P2PAddress>,
    pub committee_output: Option<GetCommitteeOutput>,
    pub peg_out_accepted: Option<PegOutAccepted>,
    pub spv_proof: Option<BtcTxSPVProof>,
    pub pegout_registered_tx: Option<String>,
    pub transaction_status: Option<TransactionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub flow_id: Uuid,
    pub step: Steps,
    pub ctx: FlowContext,
}

pub struct PegoutFlow<CG, BC, S>
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
}

impl<CG, BC, S> PegoutFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    pub fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        internal_id: Uuid,
        pegout_requested: &PegoutRequested,
        store: Rc<S>,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State {
                flow_id: internal_id,
                step: Steps::PegoutRequested,
                ctx: FlowContext {
                    pegout_requested: pegout_requested.clone(),
                    my_p2p_address: None,
                    committee_output: None,
                    peg_out_accepted: None,
                    pegout_registered_tx: None,
                    spv_proof: None,
                    transaction_status: None,
                },
            },
            store,
        }
    }

    pub fn from_saved_state(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        state: State,
        store: Rc<S>,
    ) -> Self {
        Self { contracts, rt_sync, bitvmx_broker, state, store }
    }

    fn persist_state(&self) -> Result<()> {
        debug!(
            "PegoutFlow {}: Persisting state for step: {:?}",
            self.state.flow_id, self.state.step
        );
        self.store.save_flow(&StoreKey::PegoutFlow(self.state.flow_id), self.state.clone())
    }

    pub fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.step;
        self.state.step = next_step;

        debug!(
            "PegoutFlow {}: {} -> {}",
            self.state.flow_id,
            format_step(previous_step),
            format_step(next_step)
        );

        match next_step {
            Steps::PegoutRequested => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::GetCommInfo => {
                self.request_bitvmx_comm_info()?;
            }
            //This step will prepare the user take setup by sending the setVar and setup to bitvmx in a single step to make bitvmx complete the pegout setup step.
            Steps::PrepareUserTakeSetup => {
                self.communicate_pegout_requested_to_bitvmx()?;
            }
            //In the middle of these steps the signature flow is being executed outside the flow.
            Steps::DispatchTransaction => {
                info!(
                    "Waiting for signatures to be ready to dispatch transaction for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::TriggerOperatorTake => {
                info!(
                    "Triggering operator take due to timeout for flow_id: {}",
                    self.state.flow_id
                );
                if let Err(err) = self.trigger_operator_take() {
                    warn!(
                        "Failed to trigger operator take for flow_id {}: {}. Continuing flow.",
                        self.state.flow_id, err
                    );
                }
            }
            Steps::ConfirmUserTakeTransaction => {
                info!(
                    "Waiting for transaction confirmations for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_user_take_txid()
                );
            }
            Steps::RequestUserTakeSpvProof => {
                info!(
                    "Requesting SPV proof for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_user_take_txid()
                );
                self.request_spv_proof()?;
            }
            Steps::RegisterPegout => {
                info!("Registering pegout for flow_id: {}", self.state.flow_id);
                let spv_proof = self.state.ctx.spv_proof.clone().ok_or_else(|| {
                    anyhow!(
                        "SPV proof not available for pegout registration - flow_id {}",
                        self.state.flow_id
                    )
                })?;
                let output = self.register_pegout(spv_proof.clone())?;
                self.state.ctx.pegout_registered_tx = Some(output.transaction_hash);
            }
            Steps::Done => {
                info!("PegoutFlow Done: {}", self.state.flow_id);
            }
        }

        self.persist_state()?;

        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub fn complete_step(&mut self, data: &StepData) -> Result<()> {
        let current_step: Steps = self.state.step;

        //TODO we are reaching here with done step with no sense, fix the flow to avoid this
        info!(
            "PegoutFlow {}: Completing step {} with data: {:?} for flow_id {}",
            self.state.flow_id,
            format_step(current_step),
            data,
            self.state.flow_id
        );

        // Process data and determine next state
        let next_step = self.process_step_data(current_step, data)?;

        // Transition to the next state
        self.start_step(next_step)?;

        Ok(())
    }

    fn process_step_data(&mut self, current_step: Steps, data: &StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::PegoutRequested, StepData::PegoutRequested) => Ok(Steps::GetCommInfo),
            (Steps::GetCommInfo, StepData::CommInfo(comm_info)) => {
                self.state.ctx.my_p2p_address = Some(comm_info.clone());
                Ok(Steps::PrepareUserTakeSetup)
            }
            (Steps::PrepareUserTakeSetup, StepData::PegoutAccepted(accepted)) => {
                self.state.ctx.peg_out_accepted = Some(accepted.clone());
                Ok(Steps::DispatchTransaction)
            }
            (Steps::DispatchTransaction, StepData::DispatchTransaction) => {
                self.dispatch_transaction()?;
                Ok(Steps::ConfirmUserTakeTransaction)
            }
            (Steps::DispatchTransaction, StepData::TriggerOperatorTakeTimeout) => {
                // Timeout expired, transition to TriggerOperatorTake step
                info!(
                    "Timeout expired for flow_id: {}, transitioning to TriggerOperatorTake",
                    self.state.flow_id
                );
                Ok(Steps::TriggerOperatorTake)
            }
            (Steps::TriggerOperatorTake, StepData::TriggerOperatorTakeTimeout) => {
                // After trigger_operator_take completes successfully, finish the flow
                info!(
                    //TODO improve messagec, heck this as maybe it was not successfull but is part of the flow and should be completed
                    "Operator take triggered successfully for flow_id: {}, completing flow",
                    self.state.flow_id
                );
                Ok(Steps::Done)
            }
            (Steps::ConfirmUserTakeTransaction, StepData::TransactionConfirmed(tx_status)) => {
                info!(
                    "Transaction confirmed for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id, tx_status.tx_id
                );
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
                info!("Received SPV proof for flow_id: {}", self.state.flow_id);
                trace!("SPV Proof data: {spv_proof:?}");
                self.state.ctx.spv_proof = Some(spv_proof.clone());
                Ok(Steps::RegisterPegout)
            }
            (Steps::RegisterPegout, StepData::PegoutRegistered(pegout_registered)) => {
                info!("Pegout registered successfully for flow_id: {}", self.state.flow_id);
                trace!("PegoutRegistered data: {pegout_registered:?}");
                self.send_pegout_completed_to_bitvmx(pegout_registered)?;
                Ok(Steps::Done)
            }
            _ => Err(anyhow::anyhow!(
                "Invalid state transition: {current_step:?} with data {data:?}"
            )),
        }
    }

    //This step will send the setVar and setup to bitvmx in a single step to make bitvmx complete the pegout setup step.
    fn communicate_pegout_requested_to_bitvmx(&mut self) -> Result<()> {
        info!("Communicating pegout requested to bitvmx with flow_id: {}", self.state.flow_id);
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
        debug!("Sending setup to bitvmx with flow_id: {}", self.state.flow_id);
        let committee_peer_ids = self.get_committee_peer_ids(
            self.state
                .ctx
                .committee_output
                .clone()
                .ok_or_else(|| anyhow!("Committee output not available for setup"))?,
        )?;

        let committee_addresses = self.get_committee_member_address(committee_id)?;
        let p2p_addresses = build_communication_data(
            &self
                .state
                .ctx
                .my_p2p_address
                .as_ref()
                .ok_or_else(|| anyhow!("P2P address not available for setup"))?
                .address,
            &committee_addresses,
            &committee_peer_ids,
        )?;

        let msg = IncomingBitVMXApiMessages::Setup(
            self.state.flow_id,
            PROGRAM_TYPE_USER_TAKE.to_string(),
            p2p_addresses,
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
                P2PAddressParser::addr_from_contracts(&comm_data)
                    .context("Failed to convert communication data to P2P address")
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(committee_addresses)
    }

    fn get_committee_peer_ids(
        &mut self,
        committee_output: GetCommitteeOutput,
    ) -> Result<Vec<PeerId>> {
        let mut peer_ids = Vec::new();

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

            debug!("Member {} PeerId: {:?}", member.memberAddress, key_str);
            peer_ids.push(PeerId(key_str.clone()));
        }

        Ok(peer_ids)
    }

    fn send_pegout_requested_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!("Notifying pegout requested to bitvmx with flow_id: {}", self.state.flow_id);
        let committee_output: GetCommitteeOutput =
            self.get_committee_output(committee_id.clone())?;
        self.state.ctx.committee_output = Some(committee_output.clone());
        let data_to_send: PegOutRequest = Self::pegout_requested_to_bitvmx_request(
            &self.state.ctx.pegout_requested,
            &committee_output,
        )?;

        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PegOutRequest::name().clone(),
            VariableTypes::String(serde_json::to_string(&data_to_send)?),
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn send_pegout_completed_to_bitvmx(
        &mut self,
        pegout_registered: &PegoutRegistered,
    ) -> Result<()> {
        debug!("Notifying pegout completed to bitvmx with flow_id: {}", self.state.flow_id);
        let data = serde_json::to_string(&pegout_registered)?;
        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PEGOUT_COMPLETED_VAR_NAME.to_string(),
            VariableTypes::String(data),
        );

        self.send_bitvmx_msg(msg)
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

        // Convert fixed-size hashes and ids to Vec<u8>
        // Note: v0.2.0 contracts merged pegoutSignatureHash and pegoutSignatureMessage into pegoutSignatureData struct
        let pegout_signature_hash: Vec<u8> = event.pegoutSignatureData.signatureHash.to_vec();
        let pegout_signature_message: Vec<u8> = event.pegoutSignatureData.signatureMessage.to_vec();
        let pegout_id: Vec<u8> = event.pegoutId.as_slice().to_vec();
        let slot_index =
            usize::try_from(event.slotId).map_err(|_| anyhow!("slotId too large for usize"))?;

        Ok(PegOutRequest {
            committee_id,
            stream_id: event.streamId,
            packet_number: event.packetNumber,
            slot_index,
            amount: event.amount,
            pegout_id,
            pegout_signature_hash,
            pegout_signature_message,
            user_pubkey,
            take_aggregated_key,
        })
    }

    fn build_take_aggregated_key(committee_response: &GetCommitteeOutput) -> Result<PublicKey> {
        PublicKey::from_slice(&committee_response.committee.aggregatedKey)
            .context("Failed to parse aggregated public key from committee")
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        info!("Requesting bitvmx comm info for flow_id: {}", self.state.flow_id);
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?;
        Ok(())
    }

    fn dispatch_transaction(&self) -> Result<()> {
        info!("Dispatching transaction name {} for flow_id: {}", USER_TAKE_TX, self.state.flow_id);
        let msg = IncomingBitVMXApiMessages::DispatchTransactionName(
            self.state.flow_id,
            USER_TAKE_TX.to_string(),
        );
        self.send_bitvmx_msg(msg)?;
        Ok(())
    }

    pub fn get_user_take_txid(&self) -> Option<Txid> {
        self.state.ctx.peg_out_accepted.as_ref().map(|accepted| accepted.user_take_txid)
    }

    /// Get the pegout txid from the `PegoutRequested` event
    pub fn get_pegout_txid(&self) -> String {
        hex::encode(self.state.ctx.pegout_requested.pegoutSignatureData.txid.as_slice())
    }

    fn register_pegout(&self, spv_proof: BtcTxSPVProof) -> Result<RegisterPegoutOutput> {
        let input = RegisterPegoutInput::from(spv_proof);
        let output = self
            .rt_sync
            .run(async { self.contracts.register_pegout(input).await })
            .context("Failed to register pegout with provided SPV proof")?;
        info!(
            "Pegout registration sent for flow_id {} with tx hash {}",
            self.state.flow_id, output.transaction_hash
        );
        Ok(output)
    }

    pub fn request_transaction_status(&self) -> Result<()> {
        let tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        info!(
            "Requesting transaction status for flow_id: {} and tx_id: {:?}",
            self.state.flow_id, tx_id
        );
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(self.state.flow_id, tx_id))?;
        Ok(())
    }

    pub fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))?;
        Ok(())
    }

    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done
    }

    pub fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    pub fn current_step(&self) -> Steps {
        self.state.step
    }

    pub fn _get_state(&self) -> &State {
        &self.state
    }

    pub fn pegout_requested(&self) -> &PegoutRequested {
        &self.state.ctx.pegout_requested
    }

    /// Trigger operator take when timeout expires
    fn trigger_operator_take(&self) -> Result<()> {
        let pegout_txid = self.get_pegout_txid();

        info!(
            "Calling trigger_operator_take for flow_id: {} with pegout_txid: {}",
            self.state.flow_id, pegout_txid
        );

        let input = TriggerOperatorTakeInput { pegout_txid };

        let output =
            match self.rt_sync.run(async { self.contracts.trigger_operator_take(input).await }) {
                Ok(output) => output,
                Err(domain_err) => {
                    anyhow::bail!(
                        "Failed to trigger operator take for flow_id {}: {:?}",
                        self.state.flow_id,
                        domain_err
                    );
                }
            };

        info!(
            "trigger_operator_take called successfully for flow_id {} with tx hash {}",
            self.state.flow_id, output.transaction_hash
        );

        Ok(())
    }
}

/// Helper function to format step names
fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::PegoutRequested => "Init",
        Steps::GetCommInfo => "GetCommInfo",
        Steps::PrepareUserTakeSetup => "PrepareUserTakeSetup",
        Steps::DispatchTransaction => "DispatchTransaction",
        Steps::TriggerOperatorTake => "TriggerOperatorTake",
        Steps::ConfirmUserTakeTransaction => "ValidateTransactionStatus",
        Steps::RequestUserTakeSpvProof => "RequestSpvProof",
        Steps::RegisterPegout => "RegisterPegout",
        Steps::Done => "Done",
    }
}
