use crate::flows::common::COMM_KEY_INDEX;
use crate::flows::common::GlobalContext;
use crate::flows::common::build_communication_data;
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::PeerId;
use common::msg_broker::bitvmx_types::PegOutAccepted;
use common::msg_broker::bitvmx_types::PegOutRequest;
use common::msg_broker::bitvmx_types::VariableTypes;
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, P2PAddress};
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
use transaction_dispatcher::types::{GetCommitteeInput, GetCommitteeOutput};
use union_contracts::bindings::peg_manager::PegManager::PegoutRequested;
use uuid::Uuid;

pub const PROGRAM_TYPE_USER_TAKE: &str = "take";
pub const USER_TAKE_TX: &str = "USER_TAKE_TX";

/// Steps for the pegout state machine flow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steps {
    PegoutRequested,
    GetCommInfo,
    SendSetVarToBitvmx,
    SendSetupToBitvmx,
    //TODO Check with FG about the setupCompted received and the PegoutAccepted logic. Should we wait? Check logs.
    //Process setup completed
    ProcessPegoutAccepted,
    DispatchTransaction,
    // ConfirmBitvmxTransaction,
    // RequestSPVProof,
    // RegisterPegout,
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
    PegOutRequest(PegOutRequest),
    CommitteeAddresses(Vec<P2PAddress>),
    PegoutAccepted(PegOutAccepted),
    DispatchTransaction,
    // AllNoncesReady,
    // AllSignaturesReady,
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
    global_context: GlobalContext,
}

// impl<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi, BSF: BtcSignatureSubFlowApi, FactoryBSF> std::fmt::Debug for PegoutFlow<CG, BC, BSF, FactoryBSF>{
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("PegoutFlow")
//             .field("state", &self.state)
//             .finish()
//     }
// }

impl<CG, BC> PegoutFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
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
            },
            global_context,
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
                self.request_bitvmx_comm_info();
            }
            Steps::SendSetVarToBitvmx => {
                self.send_set_var_to_bitvmx()?;
            }
            Steps::SendSetupToBitvmx => {
                self.send_setup_to_bitvmx();
            }
            Steps::ProcessPegoutAccepted => {
                self.wait_for_pegout_accepted();
            }
            Steps::DispatchTransaction => {
                info!(
                    "Waiting for signatures to be ready to dispatch transaction for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::Done => {
                info!("PegoutFlow {}: Done", self.state.flow_id);
            }
        }
        Ok(())
    }

    /// Complete the current step with data and advance to the next
    pub fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step = self.state.step;

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
                Ok(Steps::SendSetVarToBitvmx)
            }
            (Steps::SendSetVarToBitvmx, StepData::PegOutRequest(request)) => {
                let msg = IncomingBitVMXApiMessages::SetVar(
                    self.state.flow_id,
                    PegOutRequest::name().to_string(),
                    VariableTypes::String(serde_json::to_string(&request)?),
                );
                self.send_bitvmx_msg(msg);
                Ok(Steps::SendSetupToBitvmx)
            }
            (Steps::SendSetupToBitvmx, StepData::CommitteeAddresses(addresses)) => {
                let msg = IncomingBitVMXApiMessages::Setup(
                    self.state.flow_id,
                    PROGRAM_TYPE_USER_TAKE.to_string(),
                    addresses.clone(),
                    0,
                );
                self.send_bitvmx_msg(msg)?;
                Ok(Steps::ProcessPegoutAccepted)
            }
            (Steps::ProcessPegoutAccepted, StepData::PegoutAccepted(accepted)) => {
                self.state.peg_out_accepted = Some(accepted.clone());
                Ok(Steps::DispatchTransaction)
            }
            (Steps::DispatchTransaction, StepData::DispatchTransaction) => {
                self.dispatch_transaction()?;
                Ok(Steps::Done)
            }
            _ => Err(anyhow::anyhow!(
                "Invalid state transition: {:?} with data {:?}",
                current_step,
                data
            )),
        }
    }

    fn wait_for_pegout_accepted(&mut self) -> Result<()> {
        debug!(
            "Waiting for pegout accepted msg from bitvmx with flow_id: {}",
            self.state.flow_id
        );
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

    fn send_setup_to_bitvmx(&mut self) -> Result<()> {
        let committee_id: CommitteeId = self.state.pegout_requested.committeeId.try_into()?;
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

        self.complete_step(StepData::CommitteeAddresses(p2p_addresses))
    }

    fn get_committee_member_address(&mut self, committee_id: CommitteeId) -> Result<Vec<String>> {
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

    fn send_set_var_to_bitvmx(&mut self) -> Result<()> {
        debug!(
            "Notifying pegout requested to bitvmx with flow_id: {}",
            self.state.flow_id
        );

        let committee_id: CommitteeId = self.state.pegout_requested.committeeId.try_into()?;
        let committee_output: GetCommitteeOutput =
            self.get_committee_output(committee_id.clone())?;
        self.state.committee_output = Some(committee_output.clone());
        let data_to_send: PegOutRequest = self.pegout_requested_to_bitvmx_request(
            self.state.pegout_requested.clone(),
            &committee_output,
        )?;
        //TODO think about -> The step calls itself complete_step to send the PegOutRequest to BitVMX, does it have sense?
        self.complete_step(StepData::PegOutRequest(data_to_send))?;
        Ok(())
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

    fn request_bitvmx_comm_info(&self) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
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
        Steps::SendSetVarToBitvmx => "SendSetVarToBitvmx",
        Steps::SendSetupToBitvmx => "SendSetupToBitvmx",
        Steps::ProcessPegoutAccepted => "ProcessPegoutAccepted",
        Steps::DispatchTransaction => "DispatchTransaction",
        Steps::Done => "Done",
    }
}
