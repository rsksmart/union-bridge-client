use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use bitcoin::secp256k1::Parity::Even;
use bitcoin::secp256k1::XOnlyPublicKey;
use bitcoin::{PublicKey, Txid};
use common::msg_broker::bitvmx_types::{
    AdvanceFundsRequest, BtcTxSPVProof, IncomingBitVMXApiMessages, P2PAddress, ReimbursementResult,
    TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use common::types::{Address, BlockHash, BlockNumber, CommitteeId, Hash256, TxHash};
use log::{debug, info};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{GetMemberPublicKeysInput, RequestPeginInput};
use union_contracts::bindings::peg_manager::PegManager::PegoutRegistered;
use uuid::Uuid;

use crate::flows::common::TAKE_KEY_INDEX;
use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::types::OperatorTakeTriggeredEvent;

#[allow(dead_code)]
pub const PROGRAM_TYPE_ADVANCE_FUNDS: &str = "advance_funds";
#[allow(dead_code)]
pub const ADVANCE_FUNDS_TX: &str = "ADVANCE_FUNDS_TX";
#[allow(dead_code)]
pub const SELECTED_OPERATOR_PUBKEY_VAR_PREFIX: &str = "selected_operator_pubkey_";
#[allow(dead_code)]
pub const ADVANCE_FUNDS_REQUEST_VAR_NAME: &str = "advance_funds_request";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Steps {
    #[default]
    OperatorTakeTriggered,
    GetCommInfo,
    SetupAdvanceFundsProtocol,
    WaitForReimbursementResult,
    RequestOperatorTakeTx,
    RequestOperatorTakeSpvProof,
    RegisterOperatorTake,
    Done,
}

#[derive(Debug, Clone)]
pub enum StepData {
    OperatorTakeTriggered,
    CommInfo(P2PAddress),
    SetupCompleted,
    ReimbursementResult(ReimbursementResult),
    RequestOperatorTakeTx(TransactionStatus),
    SpvProof(BtcTxSPVProof),
    /// Retry register operator take without state transition data (for Native Bridge confirmation retries)
    RetryRegisterOperatorTake,
    OperatorTakeRegistered(PegoutRegistered),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OperatorTakeTriggerData {
    pub pegout_txid: Hash256,
    pub committee_id: CommitteeId,
    pub stream_id: u64,
    pub packet_number: u64,
    pub slot_id: u64,
    pub slot_index: usize,
    pub user_pubkey: PublicKey,
    pub take_operator_address: Address,
    pub take_operator_pubkey: Vec<u8>,
    pub created_at: u128,
    pub operator_take_updated_at: u128,
    pub updated_at: u128,
    pub expire_at: u128,
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
    pub tx_hash: TxHash,
}

impl OperatorTakeTriggerData {
    pub fn try_from_event(event: &OperatorTakeTriggeredEvent) -> Result<Self> {
        let inner = &event.inner;
        let pegout_txid = Hash256::from(inner.pegoutTxid);

        let committee_id = CommitteeId::from(inner.pegoutInfo.committeeId);
        let stream_id = inner.streamPosition.streamId;
        let packet_number = inner.streamPosition.packetNumber;
        let slot_id = inner.streamPosition.slotId;
        let slot_index = usize::try_from(slot_id)
            .context("Failed to convert slot id from event into usize for slot index")?;

        let user_pubkey: PublicKey = PublicKey::from_slice(inner.pegoutInfo.userPubKey.as_ref())?;

        let take_operator_address = Address::from(inner.pegoutInfo.takeOperatorAddress);
        let take_operator_pubkey = inner.pegoutInfo.takeOperatorPubKey.clone().to_vec();
        let created_at: u128 = inner
            .pegoutInfo
            .createdAt
            .try_into()
            .context("Failed to convert createdAt from OperatorTakeTriggered event")?;
        let operator_take_updated_at: u128 =
            inner.pegoutInfo.operatorTakeUpdatedAt.try_into().context(
                "Failed to convert operatorTakeUpdatedAt from OperatorTakeTriggered event",
            )?;
        let updated_at: u128 = inner
            .updatedAt
            .try_into()
            .context("Failed to convert updatedAt from OperatorTakeTriggered event")?;
        let expire_at: u128 = inner
            .expireAt
            .try_into()
            .context("Failed to convert expireAt from OperatorTakeTriggered event")?;

        Ok(Self {
            pegout_txid,
            committee_id,
            stream_id,
            packet_number,
            slot_id,
            slot_index,
            user_pubkey,
            take_operator_address,
            take_operator_pubkey,
            created_at,
            operator_take_updated_at,
            updated_at,
            expire_at,
            block_number: event.block_number,
            block_hash: event.block_hash,
            tx_hash: event.tx_hash,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FlowContext {
    pub flow_id: Uuid,
    pub step: Steps,
    pub trigger_data: OperatorTakeTriggerData,
    pub my_p2p_address: Option<P2PAddress>,
    pub committee_id_uuid: Option<Uuid>,
    pub pegin_program_id: Option<Uuid>,
    pub reimbursement_result: Option<ReimbursementResult>,
    pub operator_take_tx_id: Option<Txid>,
    pub operator_take_tx_status: Option<TransactionStatus>,
    pub spv_proof: Option<BtcTxSPVProof>,
}

pub struct AdvanceFundsFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    pub(crate) state: FlowContext,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
}

impl<CG, BC> AdvanceFundsFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        flow_id: Uuid,
        event: &OperatorTakeTriggeredEvent,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Result<Self> {
        let trigger_data = OperatorTakeTriggerData::try_from_event(event)?;

        Ok(Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: FlowContext {
                flow_id,
                step: Steps::OperatorTakeTriggered,
                trigger_data,
                my_p2p_address: None,
                committee_id_uuid: None,
                pegin_program_id: None,
                reimbursement_result: None,
                operator_take_tx_id: None,
                operator_take_tx_status: None,
                spv_proof: None,
            },
            native_bridge_verifier,
        })
    }

    pub fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.step;
        self.state.step = next_step;

        debug!(
            "AdvanceFundsFlow {}: {} -> {}",
            self.state.flow_id,
            format_step(previous_step),
            format_step(next_step)
        );

        match next_step {
            Steps::OperatorTakeTriggered => {
                unreachable!(
                    "OperatorTakeTriggered is the initial step and should not be started explicitly"
                );
            }
            Steps::GetCommInfo => {
                info!(
                    "Requesting BitVMX comm info for advance funds flow_id: {}",
                    self.state.flow_id
                );
                self.request_bitvmx_comm_info()?;
            }
            Steps::SetupAdvanceFundsProtocol => {
                info!("Setting up advance funds protocol for flow_id: {}", self.state.flow_id);
                self.setup_advance_funds_protocol()?;
            }
            Steps::WaitForReimbursementResult => {
                info!("Waiting for reimbursement result for flow_id: {}", self.state.flow_id);
                // Passive step - waiting for BitVMX to send the reimbursement result
            }
            Steps::RequestOperatorTakeTx => {
                let tx_id = self
                    .state
                    .reimbursement_result
                    .as_ref()
                    .ok_or_else(|| anyhow!("Reimbursement result not available"))?
                    .txid;
                info!(
                    "Requesting operator take transaction for flow_id: {} and txid: {}",
                    self.state.flow_id, tx_id
                );
                let pegin_program_id = self
                    .state
                    .pegin_program_id
                    .ok_or_else(|| anyhow!("Pegin program ID not available"))?;
                self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(
                    pegin_program_id,
                    tx_id,
                ))?;
            }
            Steps::RequestOperatorTakeSpvProof => {
                info!(
                    "Requesting SPV proof for operator take transaction in flow_id: {}",
                    self.state.flow_id
                );
                self.request_spv_proof()?;
            }
            Steps::RegisterOperatorTake => {
                info!("Registering operator take for flow_id: {}", self.state.flow_id);
                let spv_proof =
                    self.state.spv_proof.as_ref().ok_or_else(|| {
                        anyhow!("SPV proof not available to register operator take")
                    })?;
                self.register_operator_take(spv_proof)?;
            }
            Steps::Done => {
                info!("AdvanceFundsFlow {}: Done", self.state.flow_id);
            }
        }

        Ok(())
    }

    pub fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step = self.state.step;

        info!(
            "AdvanceFundsFlow {}: Completing step {} with data: {:?}",
            self.state.flow_id,
            format_step(current_step),
            data
        );

        let next_step = self.process_step_data(current_step, data)?;
        self.start_step(next_step)?;

        Ok(())
    }

    fn process_step_data(&mut self, current_step: Steps, data: StepData) -> Result<Steps> {
        match (current_step, data) {
            (Steps::OperatorTakeTriggered, StepData::OperatorTakeTriggered) => {
                Ok(Steps::GetCommInfo)
            }
            (Steps::GetCommInfo, StepData::CommInfo(comm_info)) => {
                self.state.my_p2p_address = Some(comm_info);
                Ok(Steps::SetupAdvanceFundsProtocol)
            }
            (Steps::SetupAdvanceFundsProtocol, StepData::SetupCompleted) => {
                Ok(Steps::WaitForReimbursementResult)
            }
            (Steps::WaitForReimbursementResult, StepData::ReimbursementResult(result)) => {
                info!(
                    "Reimbursement result received for flow_id {} - txid: {}, challenge_result: {:?}",
                    self.state.flow_id, result.txid, result.challenge_result
                );
                self.state.reimbursement_result = Some(result);
                Ok(Steps::RequestOperatorTakeTx)
            }
            (Steps::RequestOperatorTakeTx, StepData::RequestOperatorTakeTx(tx_status)) => {
                debug!(
                    "Operator take transaction received for flow_id {} - txid: {}, confirmations: {}",
                    self.state.flow_id, tx_status.tx_id, tx_status.confirmations
                );
                self.state.operator_take_tx_id = Some(tx_status.tx_id);
                self.state.operator_take_tx_status = Some(tx_status);
                Ok(Steps::RequestOperatorTakeSpvProof)
            }
            (Steps::RequestOperatorTakeSpvProof, StepData::SpvProof(spv_proof)) => {
                self.state.spv_proof = Some(spv_proof);
                Ok(Steps::RegisterOperatorTake)
            }
            (Steps::RegisterOperatorTake, StepData::RetryRegisterOperatorTake) => {
                info!("Retrying register operator take for flow_id: {}", self.state.flow_id);
                Ok(Steps::RegisterOperatorTake)
            }
            (Steps::RegisterOperatorTake, StepData::OperatorTakeRegistered(pegout_registered)) => {
                debug!(
                    "Operator take registered for flow {}: {:?}",
                    self.state.flow_id, pegout_registered
                );
                Ok(Steps::Done)
            }
            _ => Err(anyhow!("Invalid state transition from {current_step:?} with provided data",)),
        }
    }

    fn setup_advance_funds_protocol(&mut self) -> Result<()> {
        let committee_id: CommitteeId = self.state.trigger_data.committee_id.clone();
        let slot_index = self.state.trigger_data.slot_index;
        let operator_address = self.state.trigger_data.take_operator_address;

        let operator_pubkey = self.get_operator_take_pubkey()?;
        let var_name = format!("{SELECTED_OPERATOR_PUBKEY_VAR_PREFIX}{slot_index}");

        let committee_id_uuid = Uuid::from_u128(*committee_id);
        self.state.committee_id_uuid = Some(committee_id_uuid);
        debug!(
            "Publishing selected operator pubkey for committee {committee_id} slot {slot_index}",
        );
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            committee_id_uuid,
            var_name,
            VariableTypes::PubKey(operator_pubkey),
        ))?;

        let my_address = self.contracts.my_address();
        if my_address != operator_address {
            debug!(
                "Advance funds setup: node {my_address} is not selected operator (selected: {operator_address}), finishing flow",
            );
            self.start_step(Steps::Done)?;
            return Ok(());
        }

        let my_p2p_address = self
            .state
            .my_p2p_address
            .clone()
            .ok_or_else(|| anyhow!("P2P address not available for advance funds setup"))?;

        let participants = vec![my_p2p_address.clone()];

        let request_payload = self.build_advance_funds_request(operator_pubkey)?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            self.flow_id(),
            ADVANCE_FUNDS_REQUEST_VAR_NAME.to_string(),
            VariableTypes::String(request_payload),
        ))?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::Setup(
            self.flow_id(),
            PROGRAM_TYPE_ADVANCE_FUNDS.to_string(),
            participants,
            0,
        ))
    }

    fn request_spv_proof(&self) -> Result<()> {
        let tx_id = self
            .state
            .operator_take_tx_id
            .ok_or_else(|| anyhow!("Operator take transaction id not available"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))
    }

    fn register_operator_take(&self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        let input: RequestPeginInput = spv_proof.clone().into();

        invoke_contract_safe(
            &self.rt_sync,
            "registerOperatorTake",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.register_operator_take(input).await },
        )
        .context("Failed to register operator take with provided SPV proof")?;

        info!("Operator take registration sent for flow_id {}", self.state.flow_id);
        Ok(())
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo())
    }

    fn get_operator_take_pubkey(&self) -> Result<PublicKey> {
        log::info!(
            "Getting operator take public key for address: {:?}",
            self.state.trigger_data.take_operator_address
        );
        let keys = self.get_member_public_keys(self.state.trigger_data.take_operator_address)?;
        let raw_hex: &_ = keys
            .get(TAKE_KEY_INDEX)
            .context("Operator take public key missing from registry response")?;

        let key_bytes =
            hex::decode(raw_hex.trim_start_matches("0x")).context("Invalid operator pubkey hex")?;

        xonly_to_compressed_pubkey(&key_bytes)
    }

    fn build_advance_funds_request(&self, operator_pubkey: PublicKey) -> Result<String> {
        let pegout_id = self.state.trigger_data.pegout_txid.value().as_bytes().to_vec();

        let payload: AdvanceFundsRequest = AdvanceFundsRequest {
            committee_id: Uuid::from_u128(*self.state.trigger_data.committee_id),
            slot_index: self.state.trigger_data.slot_index,
            pegout_id,
            //TODO Extracted from the examples. Pending to add config for regtest vs testnet.
            fee: 335,
            user_pubkey: self.state.trigger_data.user_pubkey,
            my_take_pubkey: operator_pubkey,
        };

        serde_json::to_string(&payload)
            .context("Failed to encode advance funds request payload to JSON")
    }

    fn get_member_public_keys(&self, member_address: Address) -> Result<Vec<String>> {
        let input = GetMemberPublicKeysInput { member_address: member_address.into() };
        let response = self
            .rt_sync
            .run(async { self.contracts.get_member_public_keys(input).await })
            .context("Failed to obtain operator public keys")?;
        Ok(response.public_keys)
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        debug!("AdvanceFundsFlow sending BitVMX message: {msg:?}");
        self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?;
        Ok(())
    }

    pub fn request_transaction_status(&self) -> Result<()> {
        let tx_id = self
            .state
            .reimbursement_result
            .as_ref()
            .ok_or_else(|| anyhow!("Reimbursement result not available"))?
            .txid;

        let pegin_program_id =
            self.state.pegin_program_id.ok_or_else(|| anyhow!("Pegin program ID not available"))?;

        info!(
            "Requesting operator take transaction status for flow_id: {} and txid: {}",
            self.state.flow_id, tx_id
        );

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetTransaction(pegin_program_id, tx_id))
    }

    #[allow(dead_code)]
    pub fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    pub fn current_step(&self) -> Steps {
        self.state.step
    }

    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done
    }

    pub fn operator_take_tx_id(&self) -> Option<Txid> {
        self.state.operator_take_tx_id
    }

    pub fn committee_id_uuid(&self) -> Option<Uuid> {
        self.state.committee_id_uuid
    }

    #[allow(dead_code)]
    pub fn trigger_data(&self) -> &OperatorTakeTriggerData {
        &self.state.trigger_data
    }
}

fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::OperatorTakeTriggered => "OperatorTakeTriggered",
        Steps::GetCommInfo => "GetCommInfo",
        Steps::SetupAdvanceFundsProtocol => "SetupAdvanceFundsProtocol",
        Steps::WaitForReimbursementResult => "WaitForReimbursementResult",
        Steps::RequestOperatorTakeTx => "RequestOperatorTakeTx",
        Steps::RequestOperatorTakeSpvProof => "RequestOperatorTakeSpvProof",
        Steps::RegisterOperatorTake => "RegisterOperatorTake",
        Steps::Done => "Done",
    }
}

fn xonly_to_compressed_pubkey(bytes: &[u8]) -> Result<PublicKey> {
    let xonly =
        XOnlyPublicKey::from_slice(bytes).context("Failed to parse x-only public key bytes")?;
    let secp_pubkey = xonly.public_key(Even);
    Ok(PublicKey::new(secp_pubkey))
}
