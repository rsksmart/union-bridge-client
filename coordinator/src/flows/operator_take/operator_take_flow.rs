use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use bitcoin::PublicKey;
use bitcoin::key::Parity::Even;
use bitcoin::secp256k1::XOnlyPublicKey;
use common::msg_broker::bitvmx_types::{
    AdvanceFundsRegistered, AdvanceFundsRequest, BtcTxSPVProof, CommsAddress, FundsAdvanceSPV,
    IncomingBitVMXApiMessages, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::{Address, CommitteeId, Hash256};
use log::{debug, info, trace};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{RegisterAdvanceFundsInput, RequestPeginInput};
use union_contracts::bindings::pegout_manager::PegoutManager::PegoutRegistered;
use uuid::Uuid;

use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::types::OperatorTakeTriggeredEvent;

pub const PROGRAM_TYPE_ADVANCE_FUNDS: &str = "advance_funds";
pub const ADVANCE_FUNDS_REQUEST_VAR_NAME: &str = "advance_funds_request";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Steps {
    #[default]
    OperatorTakeTriggered,
    GetCommInfo,
    SetupAdvanceFundsProtocol,
    WaitForAdvanceFundsSPV,
    RegisterAdvanceFunds,
    WaitForAdvanceFundsRegistered,
    NotifyAdvanceFundsRegistered,
    WaitForReimbursementKickoffSpv,
    RegisterReimbursementKickoff,
    WaitForOperatorTakeSpv,
    RegisterOperatorTake,
    Done,
}

#[derive(Debug, Clone)]
pub enum StepData {
    OperatorTakeTriggered,
    CommInfo(CommsAddress),
    SetupCompleted,
    AdvanceFundsSPV(FundsAdvanceSPV),
    AdvanceFundsConfirmed(AdvanceFundsRegistered),
    AdvanceFundsNotified,
    ReimbursementKickoffSPV(BtcTxSPVProof),
    ReimbursementKickoffConfirmed,
    OperatorTakeSPV(BtcTxSPVProof),
    OperatorTakeRegistered(PegoutRegistered),
    /// Retry variants for Native Bridge confirmation retries (no state transition)
    RetryRegisterAdvanceFunds,
    RetryRegisterReimbursementKickoff,
    RetryRegisterOperatorTake,
}

#[derive(Debug, Clone)]
pub struct OperatorTakeTriggerData {
    pub pegout_txid: Hash256,
    pub pegout_id: Hash256,
    pub committee_id: CommitteeId,
    pub slot_id: u64,
    pub slot_index: usize,
    pub user_pubkey: PublicKey,
    pub take_operator_address: Address,
    pub operator_take_pubkey: PublicKey,
}

impl OperatorTakeTriggerData {
    pub fn try_from_event(event: &OperatorTakeTriggeredEvent) -> Result<Self> {
        let inner = &event.inner;
        let pegout_txid = Hash256::from(inner.pegoutTxid);
        let pegout_id = Hash256::from(inner.pegoutInfo.pegoutId);
        let committee_id = CommitteeId::from(inner.pegoutInfo.committeeId);
        let slot_id = inner.streamPosition.slotId;
        let slot_index = usize::try_from(slot_id)
            .context("Failed to convert slot id from event into usize for slot index")?;
        let user_pubkey = PublicKey::from_slice(inner.pegoutInfo.userPubKey.as_ref())?;
        let take_operator_address = Address::from(inner.pegoutInfo.takeOperatorAddress);
        let operator_take_pubkey =
            xonly_to_compressed_pubkey(inner.pegoutInfo.operatorTakePubKey.as_ref())?;
        Ok(Self {
            pegout_txid,
            pegout_id,
            committee_id,
            slot_id,
            slot_index,
            user_pubkey,
            take_operator_address,
            operator_take_pubkey,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FlowContext {
    pub flow_id: Uuid,
    pub step: Steps,
    pub trigger_data: OperatorTakeTriggerData,
    pub my_p2p_address: Option<CommsAddress>,
    pub accept_pegin_txid: Option<alloy_primitives::FixedBytes<32>>,
    pub advance_funds_spv: Option<FundsAdvanceSPV>,
    pub advance_funds_registered: Option<AdvanceFundsRegistered>,
    pub reimbursement_kickoff_spv: Option<BtcTxSPVProof>,
    pub operator_take_spv: Option<BtcTxSPVProof>,
}

pub struct AdvanceFundsFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    native_bridge_verifier: NativeBridgeVerifier<CG>,
    pub(crate) state: FlowContext,
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
        native_bridge_verifier: NativeBridgeVerifier<CG>,
        flow_id: Uuid,
        event: &OperatorTakeTriggeredEvent,
    ) -> Result<Self> {
        let trigger_data = OperatorTakeTriggerData::try_from_event(event)?;

        Ok(Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            native_bridge_verifier,
            state: FlowContext {
                flow_id,
                step: Steps::OperatorTakeTriggered,
                trigger_data,
                my_p2p_address: None,
                accept_pegin_txid: None,
                advance_funds_spv: None,
                advance_funds_registered: None,
                reimbursement_kickoff_spv: None,
                operator_take_spv: None,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        contracts: Rc<CG>,
        bitvmx_broker: Rc<BC>,
        flow_id: Uuid,
        trigger_data: OperatorTakeTriggerData,
        step: Steps,
    ) -> Self {
        Self {
            contracts,
            rt_sync: RuntimeSync::new().expect("Failed to create runtime sync for test flow"),
            bitvmx_broker,
            native_bridge_verifier: NativeBridgeVerifier::Dummy,
            state: FlowContext {
                flow_id,
                step,
                trigger_data,
                my_p2p_address: None,
                accept_pegin_txid: None,
                advance_funds_spv: None,
                advance_funds_registered: None,
                reimbursement_kickoff_spv: None,
                operator_take_spv: None,
            },
        }
    }

    #[allow(clippy::too_many_lines)]
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
            Steps::WaitForAdvanceFundsSPV => {
                if let Some(spv_data) = self.state.advance_funds_spv.clone() {
                    info!(
                        "Advance funds SPV already buffered for flow_id: {}, proceeding",
                        self.state.flow_id
                    );
                    self.complete_step(StepData::AdvanceFundsSPV(spv_data))?;
                } else {
                    info!(
                        "Waiting for advance funds SPV proof for flow_id: {}",
                        self.state.flow_id
                    );
                }
            }
            Steps::RegisterAdvanceFunds => {
                info!("Registering advance funds for flow_id: {}", self.state.flow_id);
                let spv = self
                    .state
                    .advance_funds_spv
                    .as_ref()
                    .ok_or_else(|| anyhow!("Advance funds SPV data not available"))?;
                self.register_advance_funds(&spv.spv_proof.clone())?;
                info!(
                    "Advance funds registered, waiting for on-chain confirmation for flow_id: {}",
                    self.state.flow_id
                );
                // so all selected and non-selected wait on the same step for the confirmation
                self.start_step(Steps::WaitForAdvanceFundsRegistered)?;
            }
            Steps::WaitForAdvanceFundsRegistered => {
                info!(
                    "Waiting for advance funds registration confirmation for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::NotifyAdvanceFundsRegistered => {
                info!(
                    "Notifying BitVMX of advance funds registered for flow_id: {}",
                    self.state.flow_id
                );
                self.notify_advance_funds_registered()?;
                self.complete_step(StepData::AdvanceFundsNotified)?;
            }
            Steps::WaitForReimbursementKickoffSpv => {
                if let Some(spv_proof) = self.state.reimbursement_kickoff_spv.clone() {
                    info!(
                        "Reimbursement kickoff SPV already buffered for flow_id: {}, proceeding",
                        self.state.flow_id
                    );
                    self.complete_step(StepData::ReimbursementKickoffSPV(spv_proof))?;
                } else {
                    info!(
                        "Waiting for reimbursement kickoff SPV for flow_id: {}",
                        self.state.flow_id
                    );
                }
            }
            Steps::RegisterReimbursementKickoff => {
                info!("Registering reimbursement kickoff for flow_id: {}", self.state.flow_id);
                let spv_proof = self
                    .state
                    .reimbursement_kickoff_spv
                    .as_ref()
                    .ok_or_else(|| anyhow!("Reimbursement kickoff SPV not available"))?
                    .clone();
                self.register_reimbursement_kickoff(&spv_proof)?;
                info!(
                    "Reimbursement kickoff registered, waiting for on-chain confirmation for flow_id: {}",
                    self.state.flow_id
                );
            }
            Steps::WaitForOperatorTakeSpv => {
                if let Some(spv_proof) = self.state.operator_take_spv.clone() {
                    info!(
                        "Operator take SPV already buffered for flow_id: {}, proceeding",
                        self.state.flow_id
                    );
                    self.complete_step(StepData::OperatorTakeSPV(spv_proof))?;
                } else {
                    info!(
                        "Waiting for operator take SPV proof for flow_id: {}",
                        self.state.flow_id
                    );
                }
            }
            Steps::RegisterOperatorTake => {
                info!("Registering operator take for flow_id: {}", self.state.flow_id);
                let spv_proof = self
                    .state
                    .operator_take_spv
                    .as_ref()
                    .ok_or_else(|| anyhow!("Operator take SPV not available"))?;
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
                Ok(Steps::WaitForAdvanceFundsSPV)
            }
            (Steps::WaitForAdvanceFundsSPV, StepData::AdvanceFundsSPV(spv_data)) => {
                info!(
                    "Advance funds SPV received for flow_id {} - txid: {}, committee: {}, slot: {}",
                    self.state.flow_id, spv_data.txid, spv_data.committee_id, spv_data.slot_index
                );
                self.state.advance_funds_spv = Some(spv_data);
                Ok(Steps::RegisterAdvanceFunds)
            }
            (Steps::RegisterAdvanceFunds, StepData::RetryRegisterAdvanceFunds) => {
                info!("Retrying register advance funds for flow_id: {}", self.state.flow_id);
                Ok(Steps::RegisterAdvanceFunds)
            }
            (Steps::WaitForAdvanceFundsRegistered, StepData::AdvanceFundsConfirmed(data)) => {
                self.state.advance_funds_registered = Some(data);
                Ok(Steps::NotifyAdvanceFundsRegistered)
            }
            (Steps::NotifyAdvanceFundsRegistered, StepData::AdvanceFundsNotified) => {
                Ok(Steps::WaitForReimbursementKickoffSpv)
            }
            (
                Steps::WaitForReimbursementKickoffSpv,
                StepData::ReimbursementKickoffSPV(spv_proof),
            ) => {
                info!("Reimbursement kickoff SPV received for flow_id {}", self.state.flow_id);
                self.state.reimbursement_kickoff_spv = Some(spv_proof);
                Ok(Steps::RegisterReimbursementKickoff)
            }
            (Steps::RegisterReimbursementKickoff, StepData::RetryRegisterReimbursementKickoff) => {
                info!(
                    "Retrying register reimbursement kickoff for flow_id: {}",
                    self.state.flow_id
                );
                Ok(Steps::RegisterReimbursementKickoff)
            }
            (Steps::RegisterReimbursementKickoff, StepData::ReimbursementKickoffConfirmed) => {
                Ok(Steps::WaitForOperatorTakeSpv)
            }
            (Steps::WaitForOperatorTakeSpv, StepData::OperatorTakeSPV(spv_proof)) => {
                info!("Operator take SPV received for flow_id {}", self.state.flow_id);
                self.state.operator_take_spv = Some(spv_proof);
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
        let operator_address = self.state.trigger_data.take_operator_address;
        let operator_pubkey = self.state.trigger_data.operator_take_pubkey;

        let my_address = self.contracts.my_address();
        if my_address != operator_address {
            debug!(
                "Advance funds setup: node {my_address} is not selected operator (selected: {operator_address}), waiting for confirmed registration",
            );
            self.start_step(Steps::WaitForAdvanceFundsRegistered)?;
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

    fn register_operator_take(&self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        let input: RequestPeginInput = spv_proof.clone().into();
        let output = invoke_contract_safe(
            &self.rt_sync,
            "registerOperatorTake",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.register_operator_take(input).await },
        )
        .context("Failed to register operator take with provided SPV proof")?;

        info!(
            "Operator take registration sent for flow_id {} with tx hash {}",
            self.state.flow_id, output.transaction_hash
        );

        Ok(())
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

    fn register_advance_funds(&mut self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        let input: RequestPeginInput = spv_proof.clone().into();
        let accept_pegin_txid = self.resolve_accept_pegin_txid()?;

        let register_input =
            RegisterAdvanceFundsInput { accept_pegin_txid, advance_funds_spv_proof: input };

        // Clone Rc fields to avoid conflicting borrows in the async closure
        // while `&mut self` is still in scope from resolve_accept_pegin_txid.
        let contracts = Rc::clone(&self.contracts);
        let output = invoke_contract_safe(
            &self.rt_sync,
            "registerAdvanceFunds",
            spv_proof,
            &self.native_bridge_verifier,
            || async move { contracts.register_advance_funds(register_input).await },
        )
        .context("Failed to register advance funds SPV proof")?;

        info!(
            "Advance funds registration sent for flow_id {} with tx hash {}",
            self.state.flow_id, output.transaction_hash
        );

        Ok(())
    }

    fn register_reimbursement_kickoff(&mut self, spv_proof: &BtcTxSPVProof) -> Result<()> {
        let input: RequestPeginInput = spv_proof.clone().into();
        let accept_pegin_txid = self.resolve_accept_pegin_txid()?;

        let register_input = transaction_dispatcher::types::RegisterReimbursementKickoffInput {
            accept_pegin_txid,
            kickoff_spv_proof: input,
        };

        // Clone Rc fields to avoid conflicting borrows in the async closure
        // while `&mut self` is still in scope from resolve_accept_pegin_txid.
        let contracts = Rc::clone(&self.contracts);
        let output = invoke_contract_safe(
            &self.rt_sync,
            "registerReimbursementKickoff",
            spv_proof,
            &self.native_bridge_verifier,
            || async move { contracts.register_reimbursement_kickoff(register_input).await },
        )
        .context("Failed to register reimbursement kickoff SPV proof")?;

        info!(
            "Reimbursement kickoff registration sent for flow_id {} with tx hash {}",
            self.state.flow_id, output.transaction_hash
        );

        Ok(())
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id))
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

    fn notify_advance_funds_registered(&self) -> Result<()> {
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
        ))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        trace!("AdvanceFundsFlow - sending message to BitVMX: {msg:?}");
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    pub fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    pub fn current_step(&self) -> Steps {
        self.state.step
    }

    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done
    }

    pub fn committee_id_uuid(&self) -> Uuid {
        Uuid::from_u128(*self.state.trigger_data.committee_id)
    }

    pub fn trigger_data(&self) -> &OperatorTakeTriggerData {
        &self.state.trigger_data
    }
}

fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::OperatorTakeTriggered => "OperatorTakeTriggered",
        Steps::GetCommInfo => "GetCommInfo",
        Steps::SetupAdvanceFundsProtocol => "SetupAdvanceFundsProtocol",
        Steps::WaitForAdvanceFundsSPV => "WaitForAdvanceFundsSPV",
        Steps::RegisterAdvanceFunds => "RegisterAdvanceFunds",
        Steps::WaitForAdvanceFundsRegistered => "WaitForAdvanceFundsRegistered",
        Steps::NotifyAdvanceFundsRegistered => "NotifyAdvanceFundsRegistered",
        Steps::WaitForReimbursementKickoffSpv => "WaitForReimbursementKickoffSpv",
        Steps::RegisterReimbursementKickoff => "RegisterReimbursementKickoff",
        Steps::WaitForOperatorTakeSpv => "WaitForOperatorTakeSpv",
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

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::str::FromStr;

    use alloy_primitives::FixedBytes;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{PublicKey, Transaction};
    use common::msg_broker::bitvmx_types::{
        BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::types::{Address, CommitteeId, Hash256};
    use mockall::predicate::function;
    use primitive_types::{H160, H256};
    use transaction_dispatcher::types::RegisterReimbursementKickoffInput;
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
    fn non_selected_operator_waits_for_advance_funds_registration() {
        let committee_id = Uuid::new_v4();
        let flow_id = Uuid::new_v4();
        let trigger_data = test_trigger_data(committee_id, 0);

        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(Address::from(H160::from_low_u64_be(44)));

        let broker = MockBitVmxBroker::new();

        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(broker),
            flow_id,
            trigger_data,
            Steps::GetCommInfo,
        );

        flow.start_step(Steps::SetupAdvanceFundsProtocol)
            .expect("non-selected operator should remain in passive advance funds flow");

        assert_eq!(flow.current_step(), Steps::WaitForAdvanceFundsRegistered);
    }

    #[test]
    fn entering_wait_for_reimbursement_kickoff_consumes_buffered_spv() {
        let committee_id = Uuid::new_v4();
        let flow_id = Uuid::new_v4();
        let trigger_data = test_trigger_data(committee_id, 3);

        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_register_reimbursement_kickoff()
            .with(function(move |input: &RegisterReimbursementKickoffInput| {
                input.accept_pegin_txid == FixedBytes::<32>::from([7u8; 32])
            }))
            .times(1)
            .returning(|_| {
                Ok(transaction_dispatcher::types::TxSentOutput {
                    transaction_hash: "0xdeadbeef".to_string(),
                })
            });

        let mut broker = MockBitVmxBroker::new();
        broker.expect_send().times(1).returning(|_| Ok(true));

        let mut flow = AdvanceFundsFlow::new_for_test(
            Rc::new(contracts),
            Rc::new(broker),
            flow_id,
            trigger_data,
            Steps::WaitForAdvanceFundsRegistered,
        );
        flow.state.accept_pegin_txid = Some(FixedBytes::<32>::from([7u8; 32]));
        flow.state.reimbursement_kickoff_spv = Some(test_spv_proof());

        let registered_data = AdvanceFundsRegistered {
            committee_id,
            slot_index: 3,
            txid: common::types::TxIdParser::fb_32_to_txid(FixedBytes::<32>::ZERO),
            pegout_id: vec![0u8; 32],
            operator_pubkey: PublicKey::from_str(
                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid test pubkey"),
        };

        flow.complete_step(StepData::AdvanceFundsConfirmed(registered_data))
            .expect("buffered reimbursement kickoff SPV should be consumed");

        assert_eq!(flow.current_step(), Steps::RegisterReimbursementKickoff);
    }
}
