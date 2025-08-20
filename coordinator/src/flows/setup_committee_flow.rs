use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::{
    AllCommunicationDataReadyEvent, NewCommitteePendingEvent, NewCommitteeReadyEvent,
    RskPegManagerEvents, UserRequests,
};
use alloy_primitives::{Address, FixedBytes};
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::key::Parity::Even;
use bitcoin::{PublicKey, XOnlyPublicKey};
use common::runtime_sync::RuntimeSync;
use log::{debug, error, info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, NewCommittee, OutgoingBitVMXApiMessages, P2PAddress, PartialUtxo,
    ParticipantRole, PeerId, SignedPublicKey, VariableTypes,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};

use crate::user_requests::ApplyToStream;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    CommitteeMember, CommunicationData,
};

use common::types;
use common::types::RskBlockAndUncles;
#[cfg(test)]
use mockall::automock;
use transaction_dispatcher::types::{
    ApplyToStreamInput, CommitteePublicKey, DepositAggregatedKeyInput,
    DepositCommunicationDataInput, DepositCommunicationDataOutput, GetCommunicationDataInput,
    GetMemberPublicKeysInput, GetMemberPublicKeysOutput, P2PAddressParser,
};

const NO_LEADER_IDX: u16 = 0;
const TAKE_KEY_INDEX: usize = 0;
const DISPUTE_KEY_INDEX: usize = 1;

#[cfg_attr(test, automock)]
trait SetupCommitteeFlowApi {
    fn complete_step_and_next(&mut self, req_id: Option<Uuid>, data: StepData) -> Result<()>;

    fn request_bitvmx_comm_info(&self);

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()>;

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()>;

    fn request_bitvmx_comm_pub_key(&mut self) -> Result<()>;

    fn apply_to_stream(&self) -> Result<()>;

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()>;

    fn setup_bitvmx_aggregated_dispute_pubkey(&mut self) -> Result<()>;

    fn setup_dispute_core_protocol(&mut self) -> Result<()>;
}

#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowFactoryApi<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi>
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG, BC>;
}

// TODO(iago-2) improve with structs instead of tuples, using tuples for now for validation
type PubKeyReq = Option<(Uuid, Option<SignedPublicKey>)>; // request id, response data
type AggKeyReq = Option<(Uuid, Option<PublicKey>)>; // request id, response data
type SetupCoreReq = Option<(Uuid, Uuid, Option<String>)>; // request id, committee id, response data // TODO(iago-2) TBC what to store here in data

#[derive(Default, Debug)]
struct FlowContext {
    // stepped
    user_input: Option<ApplyToStream>,
    my_comm_info: Option<P2PAddress>,
    my_take_key: PubKeyReq,
    my_dispute_key: PubKeyReq,
    my_comm_key: PubKeyReq,
    agg_take_key: AggKeyReq,
    agg_dispute_key: AggKeyReq,
    setup_core: SetupCoreReq,
    // async
    committee_pending: Option<NewCommitteePendingEvent>,
    communication_data_ready: Option<HashMap<Address, Vec<P2PAddress>>>,
    committee_ready: Option<NewCommitteeReadyEvent>,
}

impl FlowContext {
    fn get_stream_id(&self) -> Result<u8> {
        Ok(self
            .user_input
            .as_ref()
            .context("Missing stream_id")?
            .stream_id)
    }

    fn get_committee_id(&self) -> Result<Uuid> {
        let id = self
            .committee_ready
            .as_ref()
            .context("Missing committee ready event")?
            .inner
            .committeeId;

        // Big‑endian 32 bytes
        let be: [u8; 32] = id.to_be_bytes();

        // Take the *rightmost* 16 bytes (least‑significant) to retain entropy if high limbs are zero.
        let slice = &be[16..32];

        let uuid_bytes: [u8; 16] = <[u8; 16]>::try_from(slice)
            .map_err(|_| anyhow!("Expected 16 bytes, got {}", slice.len()))?;

        Ok(Uuid::from_bytes(uuid_bytes))
    }

    fn get_committee_members(&self) -> Result<Vec<CommitteeMember>> {
        let members = self
            .committee_pending
            .as_ref()
            .context("Missing committee pending event")?
            .inner
            ._committee
            .members
            .clone();

        Ok(members)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steps {
    UserRequest,
    GetMyCommInfo,
    GetMyTakeKey,
    GetDisputeKey,
    GetMyCommKey,
    ApplyToStream,
    DepositCommunicationData,
    SetupTakeAggregatedKey,
    SetupDisputeAggregatedKey,
    DepositAggregatedKey,
    SetupDisputeCoreProtocol,
    //
    Complete,
    // Optional steps
}

impl Steps {
    fn next(&self) -> Result<Steps> {
        let next = match self {
            Steps::UserRequest => Steps::GetMyCommInfo,
            Steps::GetMyCommInfo => Steps::GetMyTakeKey,
            Steps::GetMyTakeKey => Steps::GetDisputeKey,
            Steps::GetDisputeKey => Steps::GetMyCommKey,
            Steps::GetMyCommKey => Steps::ApplyToStream,
            Steps::ApplyToStream => Steps::DepositCommunicationData,
            Steps::DepositCommunicationData => Steps::SetupTakeAggregatedKey,
            Steps::SetupTakeAggregatedKey => Steps::SetupDisputeAggregatedKey,
            Steps::SetupDisputeAggregatedKey => Steps::DepositAggregatedKey,
            Steps::DepositAggregatedKey => Steps::SetupDisputeCoreProtocol,
            Steps::SetupDisputeCoreProtocol => Steps::Complete,
            Steps::Complete => {
                bail!("Flow is already complete at {:?}", self)
            }
        };

        Ok(next)
    }
}

#[derive(Debug)]
enum StepData {
    // sync or member-dependent steps
    UserRequest(ApplyToStream),
    CommInfo(P2PAddress),
    SignedPublicKey(SignedPublicKey),
    PublicKey(PublicKey),

    // async or collaborative steps
    PendingCommittee(NewCommitteePendingEvent),
    ReadyCommunicationData(AllCommunicationDataReadyEvent),
    ReadyCommittee(NewCommitteeReadyEvent),
}

impl StepData {
    fn into_user_input(self) -> Result<ApplyToStream> {
        match self {
            StepData::UserRequest(input) => Ok(input),
            _ => bail!("Expected ApplyToStreamInput"),
        }
    }

    fn into_p2p_address(self) -> Result<P2PAddress> {
        match self {
            StepData::CommInfo(addr) => Ok(addr),
            _ => bail!("Expected P2PAddress"),
        }
    }

    fn into_signed_pubkey(self) -> Result<SignedPublicKey> {
        match self {
            StepData::SignedPublicKey(pk) => Ok(pk),
            _ => bail!("Expected SignedPublicKey"),
        }
    }

    fn into_pubkey(self) -> Result<PublicKey> {
        match self {
            StepData::PublicKey(pk) => Ok(pk),
            _ => bail!("Expected PublicKey"),
        }
    }

    fn into_new_committee_pending(self) -> Result<NewCommitteePendingEvent> {
        match self {
            StepData::PendingCommittee(ev) => Ok(ev),
            _ => bail!("Expected NewCommitteePendingEvent"),
        }
    }

    fn into_all_comm_data_ready(self) -> Result<AllCommunicationDataReadyEvent> {
        match self {
            StepData::ReadyCommunicationData(ev) => Ok(ev),
            _ => bail!("Expected AllCommunicationDataReadyEvent"),
        }
    }

    fn into_new_committee_ready(self) -> Result<NewCommitteeReadyEvent> {
        match self {
            StepData::ReadyCommittee(ev) => Ok(ev),
            _ => bail!("Expected NewCommitteeReadyEvent"),
        }
    }
}

pub(crate) struct State {
    flow_id: Uuid,
    step: Steps,
    ctx: FlowContext,
}

pub(crate) struct SetupCommitteeFlow<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    _blockchain_view: Rc<RefCell<BlockchainView>>,
    state: State,
}

impl<CG, BC> SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn new(contracts: Rc<CG>, rt_sync: RuntimeSync, bitvmx_broker: Rc<BC>, flow_id: Uuid) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            _blockchain_view: Rc::new(RefCell::new(BlockchainView::new())),
            state: State {
                flow_id,
                step: Steps::UserRequest,
                ctx: FlowContext::default(),
            },
        }
    }

    fn my_address(&self) -> types::Address {
        self.contracts.my_address()
    }

    fn close_pub_key_req(
        pub_key_req: &mut PubKeyReq,
        flow_id: Uuid,
        req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<()> {
        let req_id = req_id.with_context(|| {
            format!("Missing request id on close_pub_key_req for flow {flow_id}",)
        })?;

        match pub_key_req {
            Some(r) if req_id == r.0 => {
                r.1 = Some(data.into_signed_pubkey()?);
                Ok(())
            }
            Some(r) => {
                bail!("Request id {req_id} does not match expected {r:?} for flow {flow_id}",)
            }
            None => {
                bail!("Missing request for pubkey in flow {flow_id} with req_id {req_id}",)
            }
        }
    }

    fn close_agg_key_req(
        pub_key_req: &mut AggKeyReq,
        flow_id: Uuid,
        req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<()> {
        let req_id = req_id.with_context(|| {
            format!("Missing request id on close_agg_key_req for flow {flow_id}",)
        })?;

        match pub_key_req {
            Some(r) if req_id == r.0 => {
                r.1 = Some(data.into_pubkey()?);
                Ok(())
            }
            Some(r) => {
                bail!("Request id {req_id} does not match expected {r:?} for flow {flow_id}",)
            }
            None => {
                bail!("Missing request for agg-key in flow {flow_id} with req_id {req_id}",)
            }
        }
    }

    fn add_my_comm_data(
        &self,
        communication_data: &mut HashMap<PublicKey, P2PAddress>,
    ) -> Result<()> {
        info!("Adding my communication data");

        let my_comm_pubkey = self.ctx_my_comm_key()?;
        let my_comm_info = self.ctx_my_comm_info()?;
        communication_data.insert(my_comm_pubkey.public_key, my_comm_info);

        Ok(())
    }

    fn add_other_members_comm_data(
        &self,
        _communication_data: &mut HashMap<PublicKey, P2PAddress>,
    ) -> Result<()> {
        info!("Adding other members communication data");

        // TODO add other members data
        Ok(())
    }

    fn setup_dispute_core_for_member(
        &mut self,
        _member_idx: usize,
        comittee_id: Uuid,
        _member: &CommitteeMember,
        addresses: &Vec<P2PAddress>,
    ) -> Result<()> {
        // TODO see example of the calling loop from Diego as reference: https://rootstocklabs.slack.com/archives/D07SBTC8ECS/p1753981406212199

        let _my_utxos = self.get_my_funding_utxos()?;

        // TODO SetVar of DisputeCoreData containing my utxos

        self.state.ctx.setup_core = Some((Uuid::new_v4(), comittee_id, None));

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::Setup(
            self.state.flow_id,
            "dispute_core".to_string(),
            addresses.clone(),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn request_bitvmx_member_pub_key(&self, req_id: Uuid) -> Result<()> {
        Ok(self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetSignedPubKey(req_id, true)))
    }

    fn setup_committee(&self) -> Result<()> {
        info!("Setting up committee");

        // TODO to be built by these two matching by address:
        //  - getMembersPublicKeys from the contract => pub keys
        //  - "NewPendingCommittee" event received before this step => roles
        let members: Vec<CommitteeMember> = Vec::new();

        // I will be added as the last member
        let _my_index = members.len();

        let my_role = self
            .state
            .ctx
            .user_input
            .as_ref()
            .context("User input not found in context")?
            .role
            .clone();

        let watchtower_count = members
            .iter()
            .filter(|m| m.role == u8::from(ParticipantRole::Verifier))
            .count() as u32;

        let operator_count = members
            .iter()
            .filter(|m| m.role == u8::from(ParticipantRole::Prover))
            .count() as u32;

        // build a map of communication pubkeys to addresses
        let mut communication_data = HashMap::new();
        self.add_other_members_comm_data(&mut communication_data)?;
        self.add_my_comm_data(&mut communication_data)?;

        /// TODO use this new committee struct
        // #[derive(Debug, Clone, Serialize, Deserialize)]
        // pub struct Committee {
        //     pub members: Vec<MemberData>,
        //     pub take_aggregated_key: PublicKey,
        //     pub dispute_aggregated_key: PublicKey,
        //     pub operator_count: u32,
        //     pub member_count: u32,
        //     pub packet_size: u32,
        // }

        info!("Instantiating NewCommittee");

        let new_committee = NewCommittee {
            my_role,
            take_aggregated_key: self.ctx_aggregated_take_key()?,
            dispute_aggregated_key: self.ctx_aggregated_dispute_key()?,
            addresses: communication_data,
            operator_count,
            watchtower_count,
            // TODO(iago-2) implement new contract call to get it by stream_id, less prio than the other changes as it can be hardcoded to 100 for now
            packet_size: 100,
        };

        // TODO NewCommittee contains MemberData
        // #[derive(Debug, Clone, Serialize, Deserialize)]
        // pub struct MemberData {
        //     pub role: ParticipantRole,
        //     pub take_key: PublicKey,
        //     pub dispute_key: PublicKey,
        // }

        debug!("Getting Committee ID");

        let committee_id = self
            .state
            .ctx
            .get_committee_id()
            .context("Setting up Committee")?;

        info!("SetVar NewCommittee");

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            committee_id,
            NewCommittee::name(),
            VariableTypes::String(serde_json::to_string(&new_committee)?),
        ));

        Ok(())
    }

    // TODO(iago-2) move ctx_xxx methods to FlowContext struct

    fn ctx_my_take_key(&self) -> Result<SignedPublicKey> {
        let take_data = self.state.ctx.my_take_key.as_ref().with_context(|| {
            format!(
                "Missing request for My Take key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = take_data.0;

        let signed_pubkey = take_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for My Take key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;

        Ok(signed_pubkey.clone())
    }

    fn ctx_my_dispute_key(&self) -> Result<SignedPublicKey> {
        let dispute_data = self.state.ctx.my_dispute_key.as_ref().with_context(|| {
            format!(
                "Missing request for My Dispute key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = dispute_data.0;

        let signed_pubkey = dispute_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for My Dispute key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;

        Ok(signed_pubkey.clone())
    }

    fn ctx_my_comm_key(&self) -> Result<SignedPublicKey> {
        let comm_data = self.state.ctx.my_comm_key.as_ref().with_context(|| {
            format!(
                "Missing request for My Communications key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = comm_data.0;

        let signed_pubkey = comm_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for My Communications key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;

        Ok(signed_pubkey.clone())
    }

    fn ctx_my_comm_info(&self) -> Result<P2PAddress> {
        let my_comm_info = self
            .state
            .ctx
            .my_comm_info
            .clone()
            .context("My Comm Info missing in context")?;

        Ok(my_comm_info)
    }

    fn ctx_aggregated_take_key(&self) -> Result<PublicKey> {
        let take_data = self.state.ctx.agg_take_key.as_ref().with_context(|| {
            format!(
                "Missing request for Aggregated Take key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = take_data.0;

        let pubkey = take_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for Aggregated Take key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;

        Ok(*pubkey)
    }

    fn ctx_aggregated_dispute_key(&self) -> Result<PublicKey> {
        let dispute_data = self.state.ctx.agg_dispute_key.as_ref().with_context(|| {
            format!(
                "Missing request for Aggregated Dispute key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = dispute_data.0;

        let pubkey = dispute_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for Aggregated Dispute key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;

        Ok(*pubkey)
    }

    fn ctx_member_communication_data(&self, member_addr: Address) -> Result<Vec<P2PAddress>> {
        let committee_comm_data = self
            .state
            .ctx
            .communication_data_ready
            .as_ref()
            .context("Missing communication data in context")?;

        committee_comm_data
            .get(&member_addr)
            .with_context(|| {
                format!("Missing communication data for member {member_addr} in context")
            })
            .cloned()
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        info!("Sending {msg:?} to BitVMX");

        let result = self.bitvmx_broker.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {:?}", result);
        }
    }

    fn get_my_funding_utxos(&self) -> Result<Vec<PartialUtxo>> {
        // TODO(iago-3) new contracts version includes this
        Ok(Vec::new())
    }

    fn get_member_public_keys_from_contracts(
        &mut self,
        member_address: Address,
    ) -> Result<GetMemberPublicKeysOutput> {
        self.rt_sync.run(
            self.contracts
                .get_member_public_keys(GetMemberPublicKeysInput { member_address }),
        )
    }

    fn get_communication_data_from_contracts(
        &self,
        member_address: Address,
    ) -> Result<Vec<P2PAddress>> {
        let stream_id = self
            .state
            .ctx
            .get_stream_id()
            .context("Get Communication Data")? as u64;

        let input = GetCommunicationDataInput {
            member_address,
            stream_id,
        };

        let comm_data = self
            .rt_sync
            .run(self.contracts.get_committee_communication_data(input))?;

        let mut res = comm_data
            .communication_data
            .into_iter()
            .map(|data| {
                P2PAddressParser::contracts_to_bitvmx(&data).map_err(|e| {
                    anyhow!("Could not convert CommunicationData '{data:?}' to P2PAddress: {e}",)
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // contract requires zeroed communication data for my own address on deposit, so we have to tweak it here
        if let Some(my_comm_data) = res.iter_mut().find(|data| data.address.is_empty()) {
            // if address is empty, it means it's my own communication data
            *my_comm_data = self.ctx_my_comm_info()?.clone().into();
        } else {
            bail!("Missing zeroed communication data  (mine)",);
        }

        // TODO(agus) temporary until PeerId thing is sorted out
        for i in 0..res.len() {
            if i == 0 {
                res[i].peer_id = PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100b0595a239c455f955ac2617061fadc0f3c532056da4a4ab4111b6581a62143e6c00b3041a00c290232fa65794ea0a55ca5f2ed3310ecbcab06a721d66e99a27e0d1b8a6afd8e395b741fbcf6cb73294eaeff43118f828f0118a4b5fdc95d472bcadaf2bc4d665e535ccd70b8ee5b82624794351a82c9f819d9a53638122228d1800d7d6561ae98183ae53c6cf23964c7eceeae95807db49a164cfbbc1ddc87a975fbe3d43545e8ce1bad2043cfe6a9aa3a7538ebdab8e6b900c94a691c1321d7c2d7f1a1beb3c3ef03686f7805ce938c92c8d5057cb5101cd51c1d97d7d3d4b9f13b7cb28bc5c4c5c9983a3062efc606b9c440021e1d5257d88d9c3ced0ac38f0203010001".to_string())
            } else if i == 1 {
                res[i].peer_id = PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100c96872f74e913fbcf2e068d7f508e52dad5a278123ad6546d9735e3f35163e836427ef6ea14ff28d4ca30e7f0d4e251ddf4724668675052d6adb8581550b0adb11f0dcb78a4e9d6ad00f68bf21851d590d88d9fff1d8d7678454f9df4a1daad2f8ebfe69b4ea99160a9e2d43a98cdaaaf380bc4de9f9dec6bedc9351c89c43e4d5d89abbef98664f5d57cdf5c68d93e928203c84fd038fedddac5bbe2b243378141edec442e83c57f0bab437336586f6d6bc01bee222ee8f67dfacb2d94d7a4e406d05446c9f84de055d6175217de19d1005203674b1693f1df2d3dacd11839a782c343c33e86b952740812da624f2ddfd71edf9eb5e9ddf7944b9afc3a08b2f0203010001".to_string())
            } else if i == 2 {
                res[i].peer_id = PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100e602dadfc9a2b10e6c042e10ba19628e49132fba6197f817457bd8728e881b35dc107838437b562cb9c611c2666fe3492db881630cd917178d17d21d48e664f685d9cd2ea2658501b3eb51ac7d9832e4ec580a5822616b0b663a3fb05a5aae15881baddeb7d8d329f064b460637a28ed569b93074446cb4946720474950456c950b5ae00b5f8b5a490eb1fc9af0206178ab81d3ca81b74fca1d84da9db510c10be2df4624be64fed6a6e59dc90880dc6ed61d4908ddcaf9eb0b08b0d58c5741085da051c4a537d33a8602fc22c6bef5853208698752561afa02ce763fb2bc0b88db51c90735d72dbd0ef6895c77aead64d5fe43e4d7521ed5f8da50c96636e4b0203010001".to_string())
            } else if i == 3 {
                res[i].peer_id = PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100d1f76c66923556eaa6e9db0acf025fa96049e150cccd910ed6a36d6b32e1eb531620182c34b9ec04a00ba9e2f02f6f6f1493cf0dd42ffcafe60d81c7102f7b64f22a76ebe749dd285435a4d551ed03271062318e08efafbb1e9341aabe685a56cf81abf4af7437e60e9435a0a9682f8720b3ad017c29c517c3b25cc467f5f1ccd9ab791a206cef513141938491e5527df1e615088061a7bdc19622fd43323a74020870042ce33287f730fa5d17eb7f21b1dc6bb028d2a01850b9fb3c0ae40d5023dcdd2c888691a2c50d956f8e6d3d92c3cf893388f954781d1ee118b5840ef88a0d1cc8d218e535d706b044bf6c881ceafec982fd7ed516daaab60c4ea7d15b0203010001".to_string())
            } else {
                bail!("Unexpected comm data size")
            }
        }

        Ok(res)
    }

    fn deposit_communication_data(&self) -> Result<DepositCommunicationDataOutput> {
        let stream_id = self
            .state
            .ctx
            .get_stream_id()
            .context("Deposit Communication Data")? as u64;

        info!(
            "Depositing member {} communication data for stream {stream_id}",
            self.my_address()
        );

        let mut my_comm_info = self.ctx_my_comm_info()?;

        // TODO(agus) temporary until PeerId thing is sorted out
        my_comm_info.peer_id.0.truncate(64);

        let mut communication_data = vec![];
        // communication data size
        for member in self.state.ctx.get_committee_members()? {
            let my_address: Address = self.my_address().into();
            if member.memberAddress == my_address {
                // zeroed for my own communication data according to contracts
                communication_data.push(CommunicationData::default())
            } else {
                communication_data.push(P2PAddressParser::bitvmx_to_contracts(&my_comm_info)?);
            }
        }

        self.rt_sync.run(
            self.contracts
                .deposit_communication_data(DepositCommunicationDataInput {
                    stream_id,
                    communication_data,
                }),
        )
    }

    fn close_communication_data_step(&mut self) -> Result<()> {
        let members: Vec<CommitteeMember> = self.state.ctx.get_committee_members()?;

        let comm_data_by_member: HashMap<Address, Vec<P2PAddress>> = members
            .iter()
            .map(|m| {
                let addrs = self.get_communication_data_from_contracts(m.memberAddress)?;
                Ok((m.memberAddress, addrs))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        self.state.ctx.communication_data_ready = Some(comm_data_by_member);

        Ok(())
    }

    fn deposit_aggregated_key(&self) -> Result<()> {
        let stream_id = self
            .state
            .ctx
            .get_stream_id()
            .context("Deposit Aggregated Key")? as u64;

        info!("Depositing aggregated key for stream {stream_id}");

        let aggregated_take_key = self
            .ctx_aggregated_take_key()
            .context("Deposit Aggregated Key")?;

        let x_only_key = XOnlyPublicKey::from(aggregated_take_key);
        let aggregated_key = FixedBytes::<32>::try_from(&x_only_key.serialize())
            .context("Failed to serialize aggregated public key")?;

        let input = DepositAggregatedKeyInput {
            stream_id,
            aggregated_key,
        };

        self.rt_sync
            .run(self.contracts.deposit_aggregated_key(input))?;

        Ok(())
    }

    fn get_deterministic_take_key_id(&self) -> Uuid {
        // TODO(agus) how to generate deterministic id for all clients?
        // Hardcoded UUID for take key - same on every run
        Uuid::parse_str("12345678-1234-5678-9abc-123456789abc").unwrap()
    }

    fn get_deterministic_dispute_key_id(&self) -> Uuid {
        // TODO(agus) how to generate deterministic id for all clients?
        // Hardcoded UUID for dispute key - same on every run
        Uuid::parse_str("87654321-4321-8765-cba9-987654321cba").unwrap()
    }

    fn get_committee_keys_by_type(&mut self, key_index: usize) -> Result<Vec<PublicKey>> {
        let mut committee_take_keys = vec![];

        for member in self.state.ctx.get_committee_members()? {
            let member_addr = member.memberAddress;
            let keys = self.get_member_public_keys_from_contracts(member_addr)?;
            let take_key_str = keys
                .public_keys
                .get(key_index)
                .with_context(|| format!("Take key not found on Committee for {member_addr}"))?;

            // TODO revisit this, we are encoding bytes to hex string in the contracts to then decode it back to bytes here

            let key_bytes: FixedBytes<32> = take_key_str
                .parse()
                .context("Failed to parse public key str to FixedBytes<32>")?;
            let xonly_key = XOnlyPublicKey::from_slice(key_bytes.as_slice())
                .context("Failed to parse aggregated public key")?;

            debug!("Member {member_addr} take key X: {xonly_key:?}");

            // BitVMX adjusts parity to Even, so we do the same here
            let secp_key = xonly_key.public_key(Even);
            let member_key = PublicKey::new(secp_key);
            committee_take_keys.push(member_key);
        }

        Ok(committee_take_keys)
    }
}

impl<CG, BC> SetupCommitteeFlowApi for SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn complete_step_and_next(&mut self, req_id: Option<Uuid>, data: StepData) -> Result<()> {
        let current_step = self.state.step;

        info!("Completing step {current_step:?} for req_id {req_id:?} and data {data:?}");
        debug!("Flow Context: {:?}", self.state.ctx);

        match current_step {
            Steps::UserRequest => {
                self.state.ctx.user_input = Some(data.into_user_input()?);

                self.request_bitvmx_comm_info();
            }
            Steps::GetMyCommInfo => {
                self.state.ctx.my_comm_info = Some(data.into_p2p_address()?);

                self.request_bitvmx_take_pub_key()?;
            }
            Steps::GetMyTakeKey => {
                Self::close_pub_key_req(
                    &mut self.state.ctx.my_take_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.request_bitvmx_dispute_pub_key()?;
            }
            Steps::GetDisputeKey => {
                Self::close_pub_key_req(
                    &mut self.state.ctx.my_dispute_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.request_bitvmx_comm_pub_key()?;
            }
            Steps::GetMyCommKey => {
                Self::close_pub_key_req(
                    &mut self.state.ctx.my_comm_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.apply_to_stream()?;
            }
            Steps::ApplyToStream => {
                // TODO(iago-2) sometimes it gets stuck in "successful apply to stream", investigate why

                let pending_committee = data.into_new_committee_pending()?;

                let was_selected = pending_committee.inner._committee.members.iter().any(|m| {
                    let member_addr: types::Address = m.memberAddress.into();
                    member_addr == self.my_address()
                });

                if was_selected {
                    self.state.ctx.committee_pending = Some(pending_committee);

                    self.deposit_communication_data()?;
                } else {
                    // TODO(iago-2) close the flow se we were not selected
                    bail!("Not selected for committee, flow will not continue");
                }
            }
            Steps::DepositCommunicationData => {
                data.into_all_comm_data_ready()?;

                self.close_communication_data_step()?;

                self.setup_bitvmx_aggregated_take_pubkey()?;
            }
            Steps::SetupTakeAggregatedKey => {
                Self::close_agg_key_req(
                    &mut self.state.ctx.agg_take_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.setup_bitvmx_aggregated_dispute_pubkey()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                Self::close_agg_key_req(
                    &mut self.state.ctx.agg_dispute_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.deposit_aggregated_key()?;
            }
            Steps::DepositAggregatedKey => {
                self.state.ctx.committee_ready = Some(data.into_new_committee_ready()?);

                self.setup_dispute_core_protocol()?;
            }
            Steps::SetupDisputeCoreProtocol => {
                // TODO continue flow here
            }
            Steps::Complete => {
                info!("Setup committee flow complete")
            }
        };

        let next_step = self.state.step.next()?;
        self.state.step = next_step;

        info!("Next step: {:?}", self.state.step);

        Ok(())
    }

    fn request_bitvmx_comm_info(&self) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
    }

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_take_key = Some((req_id, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_comm_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_comm_key = Some((req_id, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn apply_to_stream(&self) -> Result<()> {
        let stream_id = self
            .state
            .ctx
            .user_input
            .as_ref()
            .with_context(|| format!("Missing user input for flow {}", self.state.flow_id))?
            .stream_id;

        let role = self
            .state
            .ctx
            .user_input
            .as_ref()
            .with_context(|| format!("Missing user input for flow {}", self.state.flow_id))?
            .role
            .clone();

        let res = self
            .rt_sync
            .run(self.contracts.apply_to_stream(ApplyToStreamInput {
                stream_id,
                role: u8::from(role),
                public_keys: vec![
                    signed_to_committee_public_key(self.ctx_my_take_key()?),
                    signed_to_committee_public_key(self.ctx_my_dispute_key()?),
                    signed_to_committee_public_key(self.ctx_my_comm_key()?),
                ],
            }));

        if res.is_err() {
            bail!("Failed to apply to stream: {:?}", res);
        }

        Ok(())
    }

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_dispute_key = Some((req_id, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()> {
        info!("Setup BitVMX Aggregated Take key");

        let take_key_id = self.get_deterministic_take_key_id();
        self.state.ctx.agg_take_key = Some((take_key_id, None));

        let committee_take_keys = self.get_committee_keys_by_type(TAKE_KEY_INDEX)?;
        let communication_data = self.ctx_member_communication_data(self.my_address().into())?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            take_key_id,
            communication_data,
            Some(committee_take_keys),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn setup_bitvmx_aggregated_dispute_pubkey(&mut self) -> Result<()> {
        info!("Setup BitVMX Aggregated Dispute key");

        let dispute_key_id = self.get_deterministic_dispute_key_id();
        self.state.ctx.agg_dispute_key = Some((dispute_key_id, None));

        let committee_dispute_keys = self.get_committee_keys_by_type(DISPUTE_KEY_INDEX)?;
        let communication_data = self.ctx_member_communication_data(self.my_address().into())?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            dispute_key_id,
            communication_data,
            Some(committee_dispute_keys),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
        info!("Setting up dispute core protocol");

        // TODO complete and validate

        self.setup_committee()?;

        let committee_id = self.state.ctx.get_committee_id()?;
        let members: Vec<CommitteeMember> = self.state.ctx.get_committee_members()?;

        for (idx, member) in members.iter().enumerate() {
            let addresses = self.ctx_member_communication_data(member.memberAddress)?;
            self.setup_dispute_core_for_member(idx, committee_id, member, &addresses)?;
        }

        Ok(())
    }
}

pub(crate) struct SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, SetupCommitteeFlow<CG, BC>>,
}

impl<CG, BC, FactoryBSF> SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    pub(crate) fn new(flow_factory: FactoryBSF) -> Self {
        Self {
            flow_factory,
            flows: HashMap::new(),
        }
    }
}

impl<CG, BC, FactoryBSF> SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    fn get_first_flow_waiting_comm_info(&mut self) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // CommInfo
        self.flows
            .values_mut()
            .find(|f| f.state.step == Steps::GetMyCommInfo)
    }

    fn get_flow_for_stream_id(&mut self, stream_id: u8) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO optimize this search by keeping convenient map of stream_id -> flow_id or alike

        self.flows.values_mut().find(|f| {
            f.state
                .ctx
                .user_input
                .as_ref()
                .map_or(false, |ui| ui.stream_id == stream_id)
        })
    }
    fn get_flow_for_committee_id(
        &mut self,
        committee_id: alloy_primitives::U256,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO optimize this search by keeping convenient map of committee_id -> flow_id or alike

        // TODO(agus) for now return the first flow until NewPendingCommittee contains committeeId, we we are not testing with several committees so it will work
        self.flows.values_mut().next()

        // self.flows.values_mut().find(|f| {
        //     f.state
        //         .ctx
        //         .committee_pending
        //         .as_ref()
        //         .map_or(false, |ev| ev.inner.committeeId == committee_id)
        // })
    }

    fn get_flow_for_request_id(&mut self, uuid: &Uuid) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO super naive approach implemented here for now: find within the different flows and their step datas one with the req_id
        // an alternative could be storing all the requests (ids) for which the flow is waiting response
        // in a same array - but I find this super risky, as it will only work if a) we NEVER send 2
        // "concurrent request-id-depending" messages to BitVMX and b) BitVMX guarantees order in request/response;
        // in addition to that, any change in the code could break it and end up mixing requests/responses/steps

        self.flows.values_mut().find(|flow| {
            if let Some((req_id, _)) = &flow.state.ctx.my_take_key {
                if req_id == uuid {
                    return true;
                }
            }
            if let Some((req_id, _)) = &flow.state.ctx.my_dispute_key {
                if req_id == uuid {
                    return true;
                }
            }
            if let Some((req_id, _)) = &flow.state.ctx.my_comm_key {
                if req_id == uuid {
                    return true;
                }
            }
            if let Some((req_id, _)) = &flow.state.ctx.agg_take_key {
                if req_id == uuid {
                    return true;
                }
            }
            if let Some((req_id, _)) = &flow.state.ctx.agg_dispute_key {
                if req_id == uuid {
                    return true;
                }
            }
            if let Some((req_id, _, _)) = &flow.state.ctx.setup_core {
                if req_id == uuid {
                    return true;
                }
            }
            false
        })
    }
}

impl<CG, BC, FactoryBSF> EventProcessor for SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        info!("Processing user request: {:?}", req);
        match req {
            UserRequests::ApplyToStream(input) => {
                // TODO(iago-2) this won't be used by BitVMX, its' just for our logging, make that clear by renaming it or changing its type maybe
                let flow_id = Uuid::new_v4();
                let mut flow = self.flow_factory.create_flow(flow_id);
                flow.complete_step_and_next(None, StepData::UserRequest(input.clone()))?;
                self.flows.insert(flow_id, flow);
            }
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::NewCommitteePending(new_committee_pending) => {
                info!(
                    "Processing NewCommitteePending event: {:?}",
                    new_committee_pending
                );

                // TODO(iago-2) review this situation: should a second NewCommitteePending restart the flow? (ie. if an unmatching aggregated key is deposited, Committee gets recreated)

                let stream_id = parse_stream_id_from_u256(&new_committee_pending.inner.streamId)?;
                if let Some(first_flow) = self.get_flow_for_stream_id(stream_id) {
                    // assume confirmed for now
                    first_flow.complete_step_and_next(
                        None,
                        StepData::PendingCommittee(new_committee_pending.clone()),
                    )?
                } else {
                    bail!("No flow found for {new_committee_pending:?}")
                }
            }
            RskPegManagerEvents::AllCommunicationDataReady(all_comm_data_ready) => {
                info!(
                    "Processing AllCommunicationDataReady event: {:?}",
                    all_comm_data_ready
                );

                let stream_id = parse_stream_id_from_u64(all_comm_data_ready.inner.streamId)?;
                if let Some(first_flow) = self.get_flow_for_stream_id(stream_id) {
                    // assume confirmed for now
                    first_flow.complete_step_and_next(
                        None,
                        StepData::ReadyCommunicationData(all_comm_data_ready.clone()),
                    )?
                } else {
                    bail!("No flow found for {all_comm_data_ready:?}")
                }
            }
            RskPegManagerEvents::NewCommitteeReady(new_committee_ready) => {
                info!(
                    "Processing NewCommitteeReady event: {:?}",
                    new_committee_ready
                );

                if let Some(first_flow) =
                    self.get_flow_for_committee_id(new_committee_ready.inner.committeeId)
                {
                    // assume confirmed for now
                    first_flow.complete_step_and_next(
                        None,
                        StepData::ReadyCommittee(new_committee_ready.clone()),
                    )?
                } else {
                    bail!("No flow found for {new_committee_ready:?}")
                }
            }
            _ => {
                info!("Ignoring RSK event: {:?}", event);
            }
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::CommInfo(comm_info) => {
                // we can receive multiple CommInfo events but always for the same member of the
                // committee (the one running the client), but BitVMX will always respond with the
                // same info - so for now we send it to the first flow waiting for it
                if let Some(first_flow) = self.get_first_flow_waiting_comm_info() {
                    first_flow
                        .complete_step_and_next(None, StepData::CommInfo(comm_info.clone()))?
                } else {
                    bail!("No flow found for OutgoingBitVMXApiMessages::CommInfo")
                }
            }
            OutgoingBitVMXApiMessages::SignedPubKey(req_id, signed_key) => {
                // here I cannot get the flow by uuid with self.flows.get(uuid) because one flow
                // will have several uuids for different requests (ie. every PubKey is requested
                // with one uuid)
                if let Some(flow) = self.get_flow_for_request_id(req_id) {
                    flow.complete_step_and_next(
                        Some(*req_id),
                        StepData::SignedPublicKey(signed_key.clone()),
                    )?;
                } else {
                    bail!(
                        "No flow found for OutgoingBitVMXApiMessages::SignedPubKey and id {req_id}"
                    );
                }
            }
            OutgoingBitVMXApiMessages::AggregatedPubkey(req_id, pubkey) => {
                // Handle successful aggregated pubkey response
                if let Some(flow) = self.get_flow_for_request_id(req_id) {
                    flow.complete_step_and_next(Some(*req_id), StepData::PublicKey(*pubkey))?;
                } else {
                    bail!(
                        "No flow found for OutgoingBitVMXApiMessages::AggregatedPubkey and id {req_id}"
                    );
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // TODO(iago-2) wait for confirmations for every event, now we assume confirmed immediately
        Ok(())
    }

    fn shutdown(&mut self) {
        // TODO handle shutdown logic if necessary
    }
}

pub(crate) struct SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
}

impl<CG, BC> SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub(crate) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
        }
    }
}

// TODO commonize with other flows
impl<CG, BC> SetupCommitteeFlowFactoryApi<CG, BC> for SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn create_flow(&self, flow_id: Uuid) -> SetupCommitteeFlow<CG, BC> {
        SetupCommitteeFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            flow_id,
        )
    }
}

// Utility functions for stream ID and U256 conversions

fn parse_stream_id_from_u64(stream_id: u64) -> Result<u8> {
    u8::try_from(stream_id)
        .with_context(|| format!("Failed to convert stream_id {stream_id} to u8"))
}

fn parse_stream_id_from_u256(stream_id: &alloy_primitives::U256) -> Result<u8> {
    u8::try_from(stream_id)
        .with_context(|| format!("Failed to convert stream_id {stream_id} to u8"))
}

fn signed_to_committee_public_key(signed_pk: SignedPublicKey) -> CommitteePublicKey {
    // TODO(iago-2) this can panic, handle gracefully

    let uncompressed = signed_pk.public_key.inner.serialize_uncompressed(); // [0x04 | X(32) | Y(32)]
    let x = &uncompressed[1..33];
    let y = &uncompressed[33..65];

    CommitteePublicKey {
        x: hex::encode(x),
        y: hex::encode(y),
        r: hex::encode(&signed_pk.signature_r),
        s: hex::encode(&signed_pk.signature_s),
        v: signed_pk.recovery_id + 27, // Convert to Ethereum's v format (27 or 28)
    }
}
