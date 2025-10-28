use crate::flows::common::COMM_KEY_INDEX;
use crate::flows::common::build_communication_data;
use anyhow::{Context, Result, anyhow, bail, ensure};
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::PegOutAccepted;
use common::msg_broker::bitvmx_types::PegOutRequest;
use common::msg_broker::bitvmx_types::VariableTypes;
use common::msg_broker::bitvmx_types::{BtcTxSPVProof, IncomingBitVMXApiMessages, P2PAddress};
use common::msg_broker::bitvmx_types::{PeerId, TransactionStatus};
use common::msg_broker::broker::BROKER_SERVER_ID;
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::CommitteeId;
use log::{debug, info, trace};
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::GetCommunicationDataInput;
use transaction_dispatcher::types::GetMemberPublicKeysInput;
use transaction_dispatcher::types::P2PAddressParser;
use transaction_dispatcher::types::{
    GetCommitteeInput, GetCommitteeOutput, RegisterPegoutInput, RegisterPegoutOutput,
};
use union_contracts::bindings::peg_manager::PegManager::{PegoutRegistered, PegoutRequested};
use uuid::Uuid;

pub const PROGRAM_TYPE_USER_TAKE: &str = "take";
pub const USER_TAKE_TX: &str = "USER_TAKE_TX";
const PEOGUT_COMPLETED_VAR_NAME: &str = "PEG_OUT_COMPLETED";

/// Steps for the pegout state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steps {
    PegoutRequested,
    GetCommInfo,
    PrepareUserTakeSetup,
    //The signature flow is being executed Between these two steps and outside the flow.
    DispatchTransaction,
    ConfirmTransaction,
    RequestSpvProof,
    RegisterPegout,
    Done,
}

impl Default for Steps {
    fn default() -> Self {
        Steps::PegoutRequested
    }
}

/// Data passed between steps in the pegout flow
#[derive(Debug, Clone)]
pub enum StepData {
    PegoutRequested,
    CommInfo(P2PAddress),
    PegoutAccepted(PegOutAccepted),
    DispatchTransaction,
    TransactionConfirmed(TransactionStatus),
    SpvProof(BtcTxSPVProof),
    PegoutRegistered(PegoutRegistered),
}

/// Context for the pegout flow state machine
#[derive(Debug, Default)]
pub struct FlowContext {
    pub flow_id: Uuid,
    pub step: Steps,
    pub pegout_requested: PegoutRequested,
    pub my_p2p_address: Option<P2PAddress>,
    pub committee_output: Option<GetCommitteeOutput>,
    pub peg_out_accepted: Option<PegOutAccepted>,
    pub spv_proof: Option<BtcTxSPVProof>,
    pub pegout_registered_tx: Option<String>,
    pub transaction_status: Option<TransactionStatus>,
}

/// State machine for handling pegout flow
pub struct PegoutFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    state: FlowContext,
}

impl<CG, BC> PegoutFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        internal_id: Uuid,
        pegout_requested: PegoutRequested,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: FlowContext {
                flow_id: internal_id,
                step: Steps::PegoutRequested,
                pegout_requested: pegout_requested.clone(),
                my_p2p_address: None,
                committee_output: None,
                peg_out_accepted: None,
                pegout_registered_tx: None,
                spv_proof: None,
                transaction_status: None,
            },
        }
    }

    /// Start the next step and log the transition
    pub fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.step;
        self.state.step = next_step;

        debug!(
            "PegoutFlow {}: {} -> {}",
            self.state.flow_id,
            format_step(previous_step),
            format_step(next_step)
        );

        // Execute the entry action for the new state
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
            Steps::ConfirmTransaction => {
                info!(
                    "Waiting for transaction confirmations for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_user_take_txid()
                );
            }
            Steps::RequestSpvProof => {
                info!(
                    "Requesting SPV proof for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id,
                    self.get_user_take_txid()
                );
                self.request_spv_proof()?;
            }
            Steps::RegisterPegout => {
                info!("Registering pegout for flow_id: {}", self.state.flow_id);
                let spv_proof = self.state.spv_proof.clone().ok_or_else(|| {
                    anyhow!(
                        "SPV proof not available for pegout registration - flow_id {}",
                        self.state.flow_id
                    )
                })?;
                let output = self.register_pegout(spv_proof.clone())?;
                self.state.pegout_registered_tx = Some(output.transaction_hash);
            }
            Steps::Done => {
                info!("PegoutFlow {}: Done", self.state.flow_id);
            }
        }
        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step: Steps = self.state.step;

        info!(
            "PegoutFlow {}: Completing step {} with data: {:?} for flow_id {}",
            self.state.flow_id,
            format_step(current_step),
            data,
            self.state.flow_id
        );

        // Process data and determine next state
        let next_step = self.process_step_data(current_step, &data)?;

        // Transition to the next state
        self.start_step(next_step)?;

        Ok(())
    }

    /// Process the current step data and determine the next state
    fn process_step_data(&mut self, current_step: Steps, data: &StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::PegoutRequested, StepData::PegoutRequested) => Ok(Steps::GetCommInfo),
            (Steps::GetCommInfo, StepData::CommInfo(comm_info)) => {
                self.state.my_p2p_address = Some(comm_info.clone());
                Ok(Steps::PrepareUserTakeSetup)
            }
            (Steps::PrepareUserTakeSetup, StepData::PegoutAccepted(accepted)) => {
                self.state.peg_out_accepted = Some(accepted.clone());
                Ok(Steps::DispatchTransaction)
            }
            (Steps::DispatchTransaction, StepData::DispatchTransaction) => {
                self.dispatch_transaction()?;
                Ok(Steps::ConfirmTransaction)
            }
            (Steps::ConfirmTransaction, StepData::TransactionConfirmed(tx_status)) => {
                info!(
                    "Transaction confirmed for flow_id: {} and tx_id: {:?}",
                    self.state.flow_id, tx_status.tx_id
                );
                trace!("Transaction status data: {:?}", tx_status);
                let expected_tx_id = self
                    .get_user_take_txid()
                    .ok_or_else(|| anyhow!("Expected user take txid not found"))?;
                ensure!(
                    tx_status.tx_id == expected_tx_id,
                    "Transaction status txId mismatch: got {:?}, expected {:?}",
                    tx_status.tx_id,
                    expected_tx_id
                );
                self.state.transaction_status = Some(tx_status.clone());
                Ok(Steps::RequestSpvProof)
            }
            (Steps::RequestSpvProof, StepData::SpvProof(spv_proof)) => {
                info!("Received SPV proof for flow_id: {}", self.state.flow_id);
                trace!("SPV Proof data: {:?}", spv_proof);
                self.state.spv_proof = Some(spv_proof.clone());
                Ok(Steps::RegisterPegout)
            }
            (Steps::RegisterPegout, StepData::PegoutRegistered(pegout_registered)) => {
                info!(
                    "Pegout registered successfully for flow_id: {}",
                    self.state.flow_id
                );
                trace!("PegoutRegistered data: {:?}", pegout_registered);
                self.send_pegout_completed_to_bitvmx(pegout_registered.clone())?;
                Ok(Steps::Done)
            }
            _ => Err(anyhow::anyhow!(
                "Invalid state transition: {:?} with data {:?}",
                current_step,
                data
            )),
        }
    }

    //This step will send the setVar and setup to bitvmx in a single step to make bitvmx complete the pegout setup step.
    fn communicate_pegout_requested_to_bitvmx(&mut self) -> Result<()> {
        info!(
            "Communicating pegout requested to bitvmx with flow_id: {}",
            self.state.flow_id
        );
        let committee_id: CommitteeId = self.state.pegout_requested.committeeId.try_into()?;

        self.send_pegout_requested_to_bitvmx(&committee_id)?;
        self.send_setup_to_bitvmx(&committee_id)?;
        Ok(())
    }

    fn get_committee_output(&mut self, committee_id: CommitteeId) -> Result<GetCommitteeOutput> {
        let committee_response = self.rt_sync.run(async {
            self.contracts
                .get_committee(GetCommitteeInput { committee_id })
                .await
        })?;
        Ok(committee_response)
    }
    fn send_setup_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!(
            "Sending setup to bitvmx with flow_id: {}",
            self.state.flow_id
        );
        let committee_peer_ids = self.get_committee_peer_ids(
            self.state
                .committee_output
                .clone()
                .ok_or_else(|| anyhow!("Committee output not available for setup"))?,
        )?;

        let committee_addresses = self.get_committee_member_address(committee_id)?;
        let p2p_addresses = build_communication_data(
            self.state
                .my_p2p_address
                .as_ref()
                .ok_or_else(|| anyhow!("P2P address not available for setup"))?
                .address
                .clone(),
            committee_addresses,
            committee_peer_ids,
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
            let keys_input = GetMemberPublicKeysInput {
                member_address: member.memberAddress,
            };

            let keys_response = self
                .rt_sync
                .run(async { self.contracts.get_member_public_keys(keys_input).await })?;

            // Get the communication key (at index 2)
            let key_str = keys_response
                .public_keys
                .get(COMM_KEY_INDEX)
                .context(format!(
                    "Communication key not found for member {}",
                    member.memberAddress
                ))?;

            debug!("Member {} PeerId: {:?}", member.memberAddress, key_str);
            peer_ids.push(PeerId(key_str.to_string()));
        }

        Ok(peer_ids)
    }

    fn send_pegout_requested_to_bitvmx(&mut self, committee_id: &CommitteeId) -> Result<()> {
        debug!(
            "Notifying pegout requested to bitvmx with flow_id: {}",
            self.state.flow_id
        );
        let committee_output: GetCommitteeOutput =
            self.get_committee_output(committee_id.clone())?;
        self.state.committee_output = Some(committee_output.clone());
        let data_to_send: PegOutRequest = self.pegout_requested_to_bitvmx_request(
            self.state.pegout_requested.clone(),
            &committee_output,
        )?;

        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PegOutRequest::name().to_string(),
            VariableTypes::String(serde_json::to_string(&data_to_send)?),
        );
        self.send_bitvmx_msg(msg)?;

        Ok(())
    }

    fn send_pegout_completed_to_bitvmx(
        &mut self,
        pegout_registered: PegoutRegistered,
    ) -> Result<()> {
        debug!(
            "Notifying pegout completed to bitvmx with flow_id: {}",
            self.state.flow_id
        );
        let data = serde_json::to_string(&pegout_registered)?;
        let msg = IncomingBitVMXApiMessages::SetVar(
            self.state.flow_id,
            PEOGUT_COMPLETED_VAR_NAME.to_string(),
            VariableTypes::String(data),
        );

        self.send_bitvmx_msg(msg)
    }

    fn pegout_requested_to_bitvmx_request(
        &self,
        event: PegoutRequested,
        committee_output: &GetCommitteeOutput,
    ) -> Result<PegOutRequest> {
        debug!(
            "Preparing PegOutRequest for BitVMX from PegoutRequested event: {:?}",
            event
        );

        let committee_id: Uuid = Uuid::from_u128(event.committeeId.try_into()?);

        // Convert user pubkey bytes to bitcoin::PublicKey
        let user_pubkey = if event.userPubKey.len() == 33 {
            // Try parsing as compressed public key (33 bytes with prefix)
            debug!("Attempting to parse as compressed public key (33 bytes)");
            PublicKey::from_slice(event.userPubKey.as_ref())
                .context("Failed to parse user public key as compressed public key")?
        } else {
            bail!(
                "Invalid user public key length: {}, expected 33",
                event.userPubKey.len()
            );
        };

        let take_aggregated_key = Self::build_take_aggregated_key(&committee_output)?;

        // Convert fixed-size hashes and ids to Vec<u8>
        let pegout_signature_hash: Vec<u8> = event.pegoutSignatureHash.as_slice().to_vec();
        let pegout_id: Vec<u8> = event.pegoutId.as_slice().to_vec();
        let pegout_signature_message: Vec<u8> = event.pegoutSignatureMessage.clone().to_vec();
        let slot_index = event.slotId as usize;

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
        info!(
            "Requesting bitvmx comm info for flow_id: {}",
            self.state.flow_id
        );
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("Sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?;
        Ok(())
    }

    fn dispatch_transaction(&self) -> Result<()> {
        info!(
            "Dispatching transaction name {} for flow_id: {}",
            USER_TAKE_TX, self.state.flow_id
        );
        let msg = IncomingBitVMXApiMessages::DispatchTransactionName(
            self.state.flow_id,
            USER_TAKE_TX.to_string(),
        );
        self.send_bitvmx_msg(msg)?;
        Ok(())
    }

    pub fn get_user_take_txid(&self) -> Option<Txid> {
        self.state
            .peg_out_accepted
            .as_ref()
            .map(|accepted| accepted.user_take_txid)
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
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(
            self.state.flow_id,
            tx_id,
        ))?;
        Ok(())
    }

    pub fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .get_user_take_txid()
            .ok_or_else(|| anyhow!("Expected user take tx_id not found"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))?;
        Ok(())
    }

    /// Check if the flow is completed
    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done
    }

    /// Get the internal ID of the flow
    pub fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    /// Get the current step
    pub fn current_step(&self) -> Steps {
        self.state.step
    }
}

/// Helper function to format step names
fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::PegoutRequested => "Init",
        Steps::GetCommInfo => "GetCommInfo",
        Steps::PrepareUserTakeSetup => "PrepareUserTakeSetup",
        Steps::DispatchTransaction => "DispatchTransaction",
        Steps::ConfirmTransaction => "ValidateTransactionStatus",
        Steps::RequestSpvProof => "RequestSpvProof",
        Steps::RegisterPegout => "RegisterPegout",
        Steps::Done => "Done",
    }
}
