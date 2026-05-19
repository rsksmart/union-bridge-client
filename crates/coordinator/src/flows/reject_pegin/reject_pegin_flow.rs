use std::rc::Rc;

use anyhow::{Context, Result, anyhow, ensure};
use bitcoin::Txid;
use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, CommsAddress, IncomingBitVMXApiMessages, PROGRAM_TYPE_REJECT_PEGIN,
    RejectPeginData, TransactionStatus, VariableTypes,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types::CommitteeId;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, trace};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::RejectPeginInput;
use uuid::Uuid;

use crate::flows::common::native_bridge_verifier::{NativeBridgeVerifier, invoke_contract_safe};
use crate::store::{CoordinatorStoreApi, StoreKey};
use crate::types::RejectPeginRegisteredData;
use crate::user_requests::RejectPeginRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RejectPeginTrigger {
    pub committee_id: CommitteeId,
    pub member_index: usize,
    pub request_pegin_txid: Txid,
}

impl From<RejectPeginRequest> for RejectPeginTrigger {
    fn from(value: RejectPeginRequest) -> Self {
        Self {
            committee_id: value.committee_id,
            member_index: value.member_index,
            request_pegin_txid: value.request_pegin_txid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum Steps {
    #[default]
    GetCommInfo,
    SendRejectPegin,
    GetRejectTxConfirmation,
    GetRejectPeginSpvProof,
    RegisterRejectPeginSpv,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StepData {
    CommInfo(CommsAddress),
    SetupCompleted,
    RejectPeginTxConfirmed(TransactionStatus),
    RejectPeginSpvProof(BtcTxSPVProof),
    RetryRegisterRejectPegin,
    RejectPeginRegistered(RejectPeginRegisteredData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlowContext {
    pub protocol_id: Uuid,
    pub trigger: RejectPeginTrigger,
    pub step: Steps,
    pub my_comms_address: Option<CommsAddress>,
    pub reject_pegin_tx_status: Option<TransactionStatus>,
    pub reject_pegin_spv_proof: Option<BtcTxSPVProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct State {
    pub ctx: FlowContext,
}

pub(crate) struct RejectPeginFlow<CG, BC, S>
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
    native_bridge_verifier: NativeBridgeVerifier<CG>,
}

impl<CG, BC, S> RejectPeginFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    pub(crate) fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        trigger: RejectPeginTrigger,
        store: Rc<S>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        Self::with_protocol_id(
            contracts,
            rt_sync,
            bitvmx_broker,
            Uuid::new_v4(),
            trigger,
            store,
            native_bridge_verifier,
        )
    }

    #[cfg(test)]
    pub(crate) fn with_protocol_id(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        protocol_id: Uuid,
        trigger: RejectPeginTrigger,
        store: Rc<S>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State {
                ctx: FlowContext {
                    protocol_id,
                    trigger,
                    step: Steps::GetCommInfo,
                    my_comms_address: None,
                    reject_pegin_tx_status: None,
                    reject_pegin_spv_proof: None,
                },
            },
            store,
            native_bridge_verifier,
        }
    }

    #[cfg(not(test))]
    fn with_protocol_id(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        protocol_id: Uuid,
        trigger: RejectPeginTrigger,
        store: Rc<S>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State {
                ctx: FlowContext {
                    protocol_id,
                    trigger,
                    step: Steps::GetCommInfo,
                    my_comms_address: None,
                    reject_pegin_tx_status: None,
                    reject_pegin_spv_proof: None,
                },
            },
            store,
            native_bridge_verifier,
        }
    }

    pub(crate) fn from_saved_state(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        state: State,
        store: Rc<S>,
        native_bridge_verifier: NativeBridgeVerifier<CG>,
    ) -> Self {
        Self { contracts, rt_sync, bitvmx_broker, state, store, native_bridge_verifier }
    }

    fn persist_state(&self) -> Result<()> {
        debug!(
            "RejectPeginFlow {}: Persisting state for step: {:?}",
            self.state.ctx.protocol_id, self.state.ctx.step
        );
        self.store
            .save_flow(&StoreKey::RejectPeginFlow(self.state.ctx.protocol_id), self.state.clone())
    }

    pub(crate) fn protocol_id(&self) -> Uuid {
        self.state.ctx.protocol_id
    }

    pub(crate) fn current_step(&self) -> Steps {
        self.state.ctx.step
    }

    pub(crate) fn trigger(&self) -> &RejectPeginTrigger {
        &self.state.ctx.trigger
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state.ctx.step == Steps::Done
    }

    pub(crate) fn start(&self) -> Result<()> {
        info!(
            "RejectPeginFlow {}: starting step {}",
            self.state.ctx.protocol_id,
            format_step(self.state.ctx.step)
        );
        self.persist_state()?;
        self.request_bitvmx_comm_info()
    }

    pub(crate) fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step = self.state.ctx.step;

        info!(
            "RejectPeginFlow {}: Completing step {} with data: {:?}",
            self.state.ctx.protocol_id,
            format_step(current_step),
            data
        );

        let next_step = self.process_step_data(current_step, data)?;
        self.start_step(next_step)?;

        Ok(())
    }

    fn process_step_data(&mut self, current_step: Steps, data: StepData) -> Result<Steps> {
        debug!(
            "RejectPeginFlow {} - step {} processing data: {:?}",
            self.state.ctx.protocol_id,
            format_step(current_step),
            data
        );

        match (current_step, data) {
            (Steps::GetCommInfo, StepData::CommInfo(comm_info)) => {
                self.state.ctx.my_comms_address = Some(comm_info);
                Ok(Steps::SendRejectPegin)
            }
            (Steps::SendRejectPegin, StepData::SetupCompleted) => {
                Ok(Steps::GetRejectTxConfirmation)
            }
            (Steps::GetRejectTxConfirmation, StepData::RejectPeginTxConfirmed(tx_status)) => {
                self.update_reject_pegin_tx_status(&tx_status)?;
                Ok(Steps::GetRejectPeginSpvProof)
            }
            (Steps::GetRejectPeginSpvProof, StepData::RejectPeginSpvProof(spv_proof)) => {
                let expected_tx_id = self
                    .get_reject_pegin_txid()
                    .ok_or_else(|| anyhow!("Reject pegin tx_id not set"))?;
                ensure!(
                    spv_proof.tx.compute_txid() == expected_tx_id,
                    "Reject pegin SPV proof tx_id mismatch: got {}, expected {}",
                    spv_proof.tx.compute_txid(),
                    expected_tx_id
                );
                self.state.ctx.reject_pegin_spv_proof = Some(spv_proof);
                Ok(Steps::RegisterRejectPeginSpv)
            }
            (Steps::RegisterRejectPeginSpv, StepData::RetryRegisterRejectPegin) => {
                info!(
                    "Retrying reject pegin registration for flow_id: {}",
                    self.state.ctx.protocol_id
                );
                Ok(Steps::RegisterRejectPeginSpv)
            }
            (
                Steps::RegisterRejectPeginSpv,
                StepData::RejectPeginRegistered(reject_pegin_registered),
            ) => {
                self.validate_reject_pegin_registered(&reject_pegin_registered)?;
                info!(
                    "RejectPeginFlow {}: transitioning to Done because pegin rejection was \
                     registered on RSK for request_pegin_txid {}",
                    self.state.ctx.protocol_id, reject_pegin_registered.request_pegin_txid
                );
                Ok(Steps::Done)
            }
            _ => Err(anyhow!("Invalid state transition from {current_step:?}")),
        }
    }

    fn start_step(&mut self, next_step: Steps) -> Result<()> {
        let previous_step = self.state.ctx.step;
        self.state.ctx.step = next_step;

        debug!(
            "RejectPeginFlow {}: {} -> {}",
            self.state.ctx.protocol_id,
            format_step(previous_step),
            format_step(next_step)
        );

        match next_step {
            Steps::GetCommInfo => {
                info!(
                    "Requesting BitVMX comm info for reject pegin flow_id: {}",
                    self.state.ctx.protocol_id
                );
                self.request_bitvmx_comm_info()?;
            }
            Steps::SendRejectPegin => {
                self.send_reject_pegin_to_bitvmx()?;
            }
            Steps::GetRejectTxConfirmation => {
                info!(
                    "Waiting for reject pegin Bitcoin confirmations for flow_id: {} and tx_id: {:?}",
                    self.state.ctx.protocol_id,
                    self.get_reject_pegin_txid()
                );
            }
            Steps::GetRejectPeginSpvProof => {
                info!(
                    "Requesting reject pegin SPV proof for flow_id: {} and tx_id: {:?}",
                    self.state.ctx.protocol_id,
                    self.get_reject_pegin_txid()
                );
                self.request_spv_proof()?;
            }
            Steps::RegisterRejectPeginSpv => {
                info!(
                    "Registering reject pegin on RSK for flow_id: {}",
                    self.state.ctx.protocol_id
                );
                self.register_reject_pegin()?;
                info!(
                    "Waiting for RejectPeginRegistered event on RSK for flow_id: {} and tx_id: {:?}",
                    self.state.ctx.protocol_id,
                    self.get_reject_pegin_txid()
                );
            }
            Steps::Done => {
                info!(
                    "RejectPeginFlow {}: Done because pegin was rejected for committee {} \
                     request_pegin_txid {}",
                    self.state.ctx.protocol_id,
                    self.state.ctx.trigger.committee_id,
                    self.state.ctx.trigger.request_pegin_txid
                );
            }
        }

        self.persist_state()?;
        Ok(())
    }

    fn request_bitvmx_comm_info(&self) -> Result<()> {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(Uuid::new_v4()))
    }

    pub(crate) fn record_reject_pegin_tx_status(
        &mut self,
        tx_status: &TransactionStatus,
    ) -> Result<()> {
        self.update_reject_pegin_tx_status(tx_status)?;
        self.persist_state().context("Failed to persist reject pegin tx status")
    }

    fn update_reject_pegin_tx_status(&mut self, tx_status: &TransactionStatus) -> Result<()> {
        if let Some(existing_status) = self.state.ctx.reject_pegin_tx_status.as_ref() {
            ensure!(
                existing_status.tx_id == tx_status.tx_id,
                "Reject pegin flow {} received tx_id {} but is already tracking {}",
                self.state.ctx.protocol_id,
                tx_status.tx_id,
                existing_status.tx_id
            );
        }

        self.state.ctx.reject_pegin_tx_status = Some(tx_status.clone());
        info!(
            "Reject pegin flow {} tracking REJECT_PEGIN_TX tx_id {} confirmations {}",
            self.state.ctx.protocol_id, tx_status.tx_id, tx_status.confirmations
        );

        Ok(())
    }

    pub(crate) fn get_reject_pegin_txid(&self) -> Option<Txid> {
        self.state.ctx.reject_pegin_tx_status.as_ref().map(|status| status.tx_id)
    }

    pub(crate) fn request_transaction_status(&self) -> Result<IncomingBitVMXApiMessages> {
        let tx_id =
            self.get_reject_pegin_txid().ok_or_else(|| anyhow!("Reject pegin tx_id not set"))?;
        trace!(
            "RejectPeginFlow {}: requesting transaction status for tx_id {}",
            self.state.ctx.protocol_id, tx_id
        );
        Ok(IncomingBitVMXApiMessages::GetTransaction(self.state.ctx.protocol_id, tx_id))
    }

    fn request_spv_proof(&self) -> Result<()> {
        let tx_id =
            self.get_reject_pegin_txid().ok_or_else(|| anyhow!("Reject pegin tx_id not set"))?;
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSPVProof(tx_id))
    }

    fn register_reject_pegin(&self) -> Result<()> {
        let spv_proof = self
            .state
            .ctx
            .reject_pegin_spv_proof
            .as_ref()
            .ok_or_else(|| anyhow!("Reject pegin SPV proof not set"))?;
        let input: RejectPeginInput = spv_proof.clone().into();

        invoke_contract_safe(
            &self.rt_sync,
            "rejectPegin",
            spv_proof,
            &self.native_bridge_verifier,
            || async { self.contracts.reject_pegin(input).await },
        )
        .context("Failed to reject pegin with provided SPV proof")?;

        Ok(())
    }

    fn validate_reject_pegin_registered(
        &self,
        reject_pegin_registered: &RejectPeginRegisteredData,
    ) -> Result<()> {
        let expected_reject_pegin_txid =
            self.get_reject_pegin_txid().ok_or_else(|| anyhow!("Reject pegin tx_id not set"))?;
        ensure!(
            reject_pegin_registered.reject_pegin_txid == expected_reject_pegin_txid,
            "RejectPeginRegistered rejectPeginTxid mismatch: got {}, expected {}",
            reject_pegin_registered.reject_pegin_txid,
            expected_reject_pegin_txid
        );

        ensure!(
            reject_pegin_registered.request_pegin_txid == self.state.ctx.trigger.request_pegin_txid,
            "RejectPeginRegistered requestPeginTxid mismatch: got {}, expected {}",
            reject_pegin_registered.request_pegin_txid,
            self.state.ctx.trigger.request_pegin_txid
        );

        Ok(())
    }

    fn send_reject_pegin_to_bitvmx(&self) -> Result<()> {
        let set_var_msg = self.build_set_var_message()?;
        debug!("Starting reject pegin flow {} with {set_var_msg:?}", self.state.ctx.protocol_id);
        self.send_bitvmx_msg(set_var_msg)?;

        let setup_msg = self.build_setup_message()?;
        debug!("Continuing reject pegin flow {} with {setup_msg:?}", self.state.ctx.protocol_id);
        self.send_bitvmx_msg(setup_msg)?;

        Ok(())
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) -> Result<()> {
        self.bitvmx_broker.send(msg)?;
        Ok(())
    }

    fn build_set_var_message(&self) -> Result<IncomingBitVMXApiMessages> {
        let data = serde_json::to_string(&self.build_reject_pegin_data())
            .map_err(|e| anyhow!("Failed to serialize RejectPeginData: {e}"))?;

        Ok(IncomingBitVMXApiMessages::SetVar(
            self.state.ctx.protocol_id,
            RejectPeginData::name().to_string(),
            VariableTypes::String(data),
        ))
    }

    fn build_setup_message(&self) -> Result<IncomingBitVMXApiMessages> {
        Ok(IncomingBitVMXApiMessages::Setup(
            self.state.ctx.protocol_id,
            PROGRAM_TYPE_REJECT_PEGIN.to_string(),
            vec![
                self.state
                    .ctx
                    .my_comms_address
                    .clone()
                    .context("Reject pegin comm info missing in flow context")?,
            ],
            0_u16,
        ))
    }

    fn build_reject_pegin_data(&self) -> RejectPeginData {
        RejectPeginData {
            committee_id: Uuid::from_u128(*self.state.ctx.trigger.committee_id),
            member_index: self.state.ctx.trigger.member_index,
            txid: self.state.ctx.trigger.request_pegin_txid,
        }
    }
}

fn format_step(step: Steps) -> &'static str {
    match step {
        Steps::GetCommInfo => "GetCommInfo",
        Steps::SendRejectPegin => "SendRejectPegin",
        Steps::GetRejectTxConfirmation => "GetRejectTxConfirmation",
        Steps::GetRejectPeginSpvProof => "GetRejectPeginSpvProof",
        Steps::RegisterRejectPeginSpv => "RegisterRejectPeginSpv",
        Steps::Done => "Done",
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::rc::Rc;

    use bitcoin::Transaction;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use common::msg_broker::bitvmx_types::{
        IncomingBitVMXApiMessages, PROGRAM_TYPE_REJECT_PEGIN, RejectPeginData,
        TransactionBlockchainStatus, VariableTypes,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use mockall::Sequence;
    use mockall::predicate::function;
    use transaction_dispatcher::types::RejectPeginInput;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::store::CoordinatorStore;

    type BitVmxMock = MockBrokerClientApi<
        IncomingBitVMXApiMessages,
        common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages,
    >;
    type MockRejectPeginFlow =
        RejectPeginFlow<MockRskContractsGatewayApi, BitVmxMock, CoordinatorStore>;

    fn test_comms_address() -> CommsAddress {
        CommsAddress {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 61180),
            pubkey_hash: "ab".repeat(32),
        }
    }

    fn test_trigger() -> RejectPeginTrigger {
        RejectPeginTrigger {
            committee_id: CommitteeId::from(42_u128),
            member_index: 3,
            request_pegin_txid: "0707070707070707070707070707070707070707070707070707070707070707"
                .parse()
                .expect("valid txid"),
        }
    }

    fn test_tx_status(txid: Txid, confirmations: u32) -> TransactionStatus {
        TransactionStatus {
            tx_id: txid,
            tx: Transaction {
                version: Version::TWO,
                lock_time: LockTime::ZERO,
                input: vec![],
                output: vec![],
            },
            block_info: None,
            confirmations,
            status: TransactionBlockchainStatus::Confirmed,
        }
    }

    fn test_spv_proof() -> BtcTxSPVProof {
        let tx_status = test_tx_status(
            "abababababababababababababababababababababababababababababababab"
                .parse()
                .expect("valid txid"),
            10,
        );
        BtcTxSPVProof {
            block_hash: "00".repeat(32),
            tx: tx_status.tx,
            merkle_branch_path: "0".to_string(),
            merkle_branch_hashes: vec![],
        }
    }

    fn test_store() -> Rc<CoordinatorStore> {
        let path = std::env::temp_dir().join(format!("reject-pegin-flow-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp store dir");
        Rc::new(CoordinatorStore::new(path.to_str().expect("utf8 path")).expect("store"))
    }

    fn test_flow(
        contracts: MockRskContractsGatewayApi,
        bitvmx_broker: BitVmxMock,
    ) -> MockRejectPeginFlow {
        MockRejectPeginFlow::with_protocol_id(
            Rc::new(contracts),
            RuntimeSync::new().expect("runtime"),
            Rc::new(bitvmx_broker),
            Uuid::new_v4(),
            test_trigger(),
            test_store(),
            NativeBridgeVerifier::Dummy,
        )
    }

    #[test]
    fn start_requests_comm_info() {
        let protocol_id = Uuid::new_v4();
        let mut bitvmx_broker = BitVmxMock::new();

        bitvmx_broker
            .expect_send()
            .with(function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::GetCommInfo(_))
            }))
            .times(1)
            .returning(|_| Ok(true));

        let flow = MockRejectPeginFlow::with_protocol_id(
            Rc::new(MockRskContractsGatewayApi::new()),
            RuntimeSync::new().expect("runtime"),
            Rc::new(bitvmx_broker),
            protocol_id,
            test_trigger(),
            test_store(),
            NativeBridgeVerifier::Dummy,
        );

        flow.start().expect("reject pegin start succeeds");
    }

    #[test]
    fn comm_info_sends_set_var_then_setup() {
        let protocol_id = Uuid::new_v4();
        let trigger = test_trigger();
        let my_comms_address = test_comms_address();

        let expected_payload = serde_json::to_string(&RejectPeginData {
            committee_id: Uuid::from_u128(*trigger.committee_id),
            member_index: trigger.member_index,
            txid: trigger.request_pegin_txid,
        })
        .expect("payload serialization");

        let mut sequence = Sequence::new();
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker
            .expect_send()
            .with(function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::SetVar(id, name, VariableTypes::String(payload))
                        if *id == protocol_id
                            && name == RejectPeginData::name()
                            && payload == &expected_payload
                )
            }))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(true));
        bitvmx_broker
            .expect_send()
            .with(function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(
                    msg,
                    IncomingBitVMXApiMessages::Setup(id, program_type, participants, leader)
                        if *id == protocol_id
                            && program_type == PROGRAM_TYPE_REJECT_PEGIN
                            && participants == &vec![my_comms_address.clone()]
                            && *leader == 0_u16
                )
            }))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(true));

        let mut flow = MockRejectPeginFlow::with_protocol_id(
            Rc::new(MockRskContractsGatewayApi::new()),
            RuntimeSync::new().expect("runtime"),
            Rc::new(bitvmx_broker),
            protocol_id,
            trigger,
            test_store(),
            NativeBridgeVerifier::Dummy,
        );

        flow.complete_step(StepData::CommInfo(test_comms_address()))
            .expect("comm info should advance flow");

        assert_eq!(flow.current_step(), Steps::SendRejectPegin);
        assert!(!flow.is_done());
    }

    #[test]
    fn setup_completed_moves_flow_to_confirmation_step() {
        let mut flow = test_flow(MockRskContractsGatewayApi::new(), BitVmxMock::new());
        flow.state.ctx.step = Steps::SendRejectPegin;

        flow.complete_step(StepData::SetupCompleted).expect("setup completion should advance flow");

        assert_eq!(flow.current_step(), Steps::GetRejectTxConfirmation);
        assert!(!flow.is_done());
    }

    #[test]
    fn confirmed_reject_pegin_tx_requests_spv_proof() {
        let txid = "abababababababababababababababababababababababababababababababab"
            .parse()
            .expect("valid txid");
        let mut bitvmx_broker = BitVmxMock::new();
        bitvmx_broker
            .expect_send()
            .with(function(move |msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::GetSPVProof(id) if *id == txid)
            }))
            .times(1)
            .returning(|_| Ok(true));

        let mut flow = test_flow(MockRskContractsGatewayApi::new(), bitvmx_broker);
        flow.state.ctx.step = Steps::GetRejectTxConfirmation;

        let tx_status = test_tx_status(txid, 12);
        flow.record_reject_pegin_tx_status(&tx_status).expect("tx status is buffered");
        flow.complete_step(StepData::RejectPeginTxConfirmed(tx_status))
            .expect("confirmed tx should request SPV proof");

        assert_eq!(flow.current_step(), Steps::GetRejectPeginSpvProof);
        assert_eq!(flow.get_reject_pegin_txid(), Some(txid));
    }

    fn test_reject_pegin_registered(
        txid: Txid,
        request_pegin_txid: Txid,
    ) -> RejectPeginRegisteredData {
        RejectPeginRegisteredData {
            reject_pegin_txid: txid,
            request_pegin_txid,
            stream_id: 42,
            packet_number: 33,
            slot_id: 3,
            peg_status: 2,
        }
    }

    #[test]
    fn spv_proof_registers_reject_pegin_and_waits_for_rsk_event() {
        let spv_proof = test_spv_proof();
        let txid = spv_proof.tx.compute_txid();
        let request_pegin_txid = test_trigger().request_pegin_txid;

        let mut contracts = MockRskContractsGatewayApi::new();
        contracts
            .expect_reject_pegin()
            .with(function(move |input: &RejectPeginInput| {
                input.block_hash == "00".repeat(32)
                    && input.btc_tx.lock_time == 0
                    && input.merkle_branch_path == "0"
            }))
            .times(1)
            .returning(|_| {
                Ok(transaction_dispatcher::types::TxSentOutput {
                    transaction_hash: "0xdeadbeef".to_string(),
                })
            });

        let mut flow = test_flow(contracts, BitVmxMock::new());
        flow.state.ctx.step = Steps::GetRejectPeginSpvProof;
        flow.state.ctx.reject_pegin_tx_status = Some(test_tx_status(txid, 12));

        flow.complete_step(StepData::RejectPeginSpvProof(spv_proof))
            .expect("SPV proof should register reject pegin");

        assert!(!flow.is_done());
        assert_eq!(flow.current_step(), Steps::RegisterRejectPeginSpv);

        flow.complete_step(StepData::RejectPeginRegistered(test_reject_pegin_registered(
            txid,
            request_pegin_txid,
        )))
        .expect("RejectPeginRegistered should complete the flow");

        assert!(flow.is_done());
        assert_eq!(flow.current_step(), Steps::Done);
    }
}
