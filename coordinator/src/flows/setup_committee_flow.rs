use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::{
    AllCommunicationDataReadyEvent, NewCommitteePendingEvent, NewCommitteeReadyEvent,
    RskPegManagerEvents, UserRequests,
};
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::PublicKey;
use bitcoin::hex::DisplayHex;
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
    CommitteeMember, NewPendingCommittee,
};

use common::types::RskBlockAndUncles;
#[cfg(test)]
use mockall::automock;
use transaction_dispatcher::types::{
    ApplyToStreamInput, CommitteePublicKey, DepositCommunicationDataInput,
    DepositCommunicationDataOutput, GetMemberCommunicationDataOutput, GetMemberPublicKeysInput,
    GetMemberPublicKeysOutput, P2PAddressParser,
};

const NO_LEADER_IDX: u16 = 0;

// TODO(iago) create proper type and mapping
type Member = CommitteeMember;

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

// TODO(iago) improve with structs instead of tuples, using tuples for now for validation
type PubKeyReq = Option<(Uuid, Option<SignedPublicKey>)>; // request id, response data
type AggKeyReq = Option<(Uuid, Option<PublicKey>)>; // request id, response data
type SetupCoreReq = Option<(Uuid, Uuid, Option<String>)>; // request id, committee id, response data // TODO(iago) TBC what to store here in data

#[derive(Default, Debug)]
struct FlowContext {
    user_input: Option<ApplyToStream>,
    my_comm_info: Option<P2PAddress>,
    my_take_key: PubKeyReq,
    my_dispute_key: PubKeyReq,
    my_comm_key: PubKeyReq,
    agg_take_key: AggKeyReq,
    agg_dispute_key: AggKeyReq,
    setup_core: SetupCoreReq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steps {
    UserRequest,
    GetMyCommInfo,
    GetMyTakeKey,
    GetDisputeKey,
    GetMyCommKey,
    ApplyToStream,
    SetupTakeAggregatedKey,
    SetupDisputeAggregatedKey,
    SetupDisputeCoreProtocol,
    //
    Complete,
}

impl Steps {
    fn next(&self) -> Result<Steps> {
        let next = match self {
            Steps::UserRequest => Steps::GetMyCommInfo,
            Steps::GetMyCommInfo => Steps::GetMyTakeKey,
            Steps::GetMyTakeKey => Steps::GetDisputeKey,
            Steps::GetDisputeKey => Steps::GetMyCommKey,
            Steps::GetMyCommKey => Steps::ApplyToStream,
            Steps::ApplyToStream => Steps::SetupTakeAggregatedKey,
            Steps::SetupTakeAggregatedKey => Steps::SetupDisputeAggregatedKey,
            Steps::SetupDisputeAggregatedKey => Steps::SetupDisputeCoreProtocol,
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
    UserRequest(ApplyToStream),
    CommInfo(P2PAddress),
    SignedPublicKey(SignedPublicKey),
    PublicKey(PublicKey),
    ApplyToStreamConfirmed,
    NewCommitteePending(NewCommitteePendingEvent),
    AllCommunicationDataReady(AllCommunicationDataReadyEvent),
    NewCommitteeReady(NewCommitteeReadyEvent),
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
        let event = match self {
            StepData::NewCommitteePending(event) => event,
            _ => return Err(anyhow!("Expected NewCommitteePending")),
        };

        // TODO(iago) this will come from the event directly
        let committee_id = format!("{}-{}", event.inner.streamId, event.block_number);

        Ok(event)
    }

    fn into_new_committee_ready(self) -> Result<NewCommitteeReadyEvent> {
        let event = match self {
            StepData::NewCommitteeReady(event) => event,
            _ => return Err(anyhow!("Expected NewCommitteeReady")),
        };

        // TODO(agus) we can use it to create the committee_id
        let block_num = event.block_number;

        Ok(event)
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
    blockchain_view: Rc<RefCell<BlockchainView>>,
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
            blockchain_view: Rc::new(RefCell::new(BlockchainView::new())),
            state: State {
                flow_id,
                step: Steps::UserRequest,
                ctx: FlowContext::default(),
            },
        }
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

        // TODO(Fairgate) awaiting Fairgate to add it to the API
        let my_comm_pubkey = self.ctx_my_comm_key()?;

        let my_comm_address = self
            .state
            .ctx
            .my_comm_info
            .as_ref()
            .context("P2P address not found in context")?
            .clone();

        communication_data.insert(my_comm_pubkey, my_comm_address);

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

    fn setup_aggregated_pubkey(&self, keys: Vec<PublicKey>) -> Result<()> {
        // TODO(Fairgate) confirm with Fairgate how to get this, if it's from the pending committee or how

        // Use hardcoded operator addresses
        let addresses = self.get_hardcoded_operators_addresses();

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            self.state.flow_id,
            addresses,
            Some(keys),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn setup_dispute_core_for_member(
        &mut self,
        member_idx: usize,
        comittee_id: Uuid,
        member: &Member,
        addresses: &Vec<P2PAddress>,
    ) -> Result<()> {
        // TODO(iago) see example of the calling loop from Diego as reference: https://rootstocklabs.slack.com/archives/D07SBTC8ECS/p1753981406212199

        let my_utxos = self.get_my_funding_utxos()?;

        // TODO(iago) SetVar of DisputeCoreData containing my utxos

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

    fn setup_committee(&self) -> Result<Uuid> {
        info!("Setting up committee");

        // TODO(iago) to be built by these two matching by address:
        //  - getMembersPublicKeys from the contract => pub keys
        //  - "NewPendingCommittee" event received before this step => roles
        let members: Vec<Member> = Vec::new();

        // I will be added as the last member
        let my_index = members.len();

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

        /// TODO(iago) use this new committee struct
        // #[derive(Debug, Clone, Serialize, Deserialize)]
        // pub struct Committee {
        //     pub members: Vec<MemberData>,
        //     pub take_aggregated_key: PublicKey,
        //     pub dispute_aggregated_key: PublicKey,
        //     pub operator_count: u32,
        //     pub member_count: u32,
        //     pub packet_size: u32,
        // }
        let new_committee = NewCommittee {
            my_role,
            take_aggregated_key: self.ctx_aggregated_take_key()?,
            dispute_aggregated_key: self.ctx_aggregated_dispute_key()?,
            addresses: communication_data,
            operator_count,
            watchtower_count,
            // TODO(iago) implement new contract call to get it by stream_id, less prio than the other changes as it can be hardcoded to 100 for now
            packet_size: 100,
        };

        // TODO(iago) NewCommittee contains MemberData
        // #[derive(Debug, Clone, Serialize, Deserialize)]
        // pub struct MemberData {
        //     pub role: ParticipantRole,
        //     pub take_key: PublicKey,
        //     pub dispute_key: PublicKey,
        // }

        // TODO(iago) store it in the context of this step and use it to relate the response
        let committee_id = Uuid::new_v4();

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            committee_id,
            NewCommittee::name(),
            VariableTypes::String(serde_json::to_string(&new_committee)?),
        ));

        Ok(committee_id)
    }

    fn get_members_from_contract(&self) -> Result<Vec<Member>> {
        info!("Getting members from contract");

        // TODO(iago) call contracts
        Ok(Vec::new())
    }

    fn ctx_my_take_key(&self) -> Result<PublicKey> {
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

        Ok(signed_pubkey.public_key)
    }

    fn ctx_my_dispute_key(&self) -> Result<PublicKey> {
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

        Ok(signed_pubkey.public_key)
    }

    fn ctx_my_comm_key(&self) -> Result<PublicKey> {
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

        Ok(signed_pubkey.public_key)
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

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        info!("Sending {msg:?} to BitVMX");

        let result = self.bitvmx_broker.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {:?}", result);
        }
    }

    fn get_my_funding_utxos(&self) -> Result<Vec<PartialUtxo>> {
        // TODO(Fairgate) Diego is working on this, ask again in some days
        Ok(Vec::new())
    }

    fn ctx_user_input(&self) -> Result<ApplyToStream> {
        self.state
            .ctx
            .user_input
            .as_ref()
            .with_context(|| format!("Missing user input for flow {}", self.state.flow_id))
            .map(|input| input.clone())
    }

    fn get_member_public_keys_from_contracts(
        &mut self,
        address: Address,
    ) -> Result<GetMemberPublicKeysOutput> {
        self.rt_sync.run(
            self.contracts
                .get_member_public_keys(GetMemberPublicKeysInput {
                    member_address: address,
                }),
        )
    }

    fn get_my_communication_data_from_contracts(
        &mut self,
        stream_id: u64,
    ) -> Result<Vec<P2PAddress>> {
        let comm_data = self
            .rt_sync
            .run(self.contracts.get_committee_communication_data(stream_id))?;

        let res = comm_data
            .communication_data
            .into_iter()
            .map(|data| {
                P2PAddressParser::contracts_to_bitvmx(&data).map_err(|e| {
                    anyhow!("Could not convert CommunicationData '{data:?}' to P2PAddress: {e}",)
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(res)
    }

    fn deposit_communication_data_from_contracts(
        &self,
        stream_id: u64,
        p2p_addrs: Vec<P2PAddress>,
    ) -> Result<DepositCommunicationDataOutput> {
        let communication_data = p2p_addrs
            .into_iter()
            .map(|addr| P2PAddressParser::bitvmx_to_contracts(&addr))
            .collect::<Result<Vec<_>>>()?;

        self.rt_sync.run(
            self.contracts
                .deposit_communication_data(DepositCommunicationDataInput {
                    stream_id,
                    communication_data,
                }),
        )
    }

    fn ctx_signed_take_key(&self) -> Result<SignedPublicKey> {
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

    fn ctx_signed_dispute_key(&self) -> Result<SignedPublicKey> {
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

    fn ctx_signed_comm_key(&self) -> Result<SignedPublicKey> {
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

    fn to_hex_prefixed(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(2 + bytes.len() * 2);
        s.push_str("0x");
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut s, "{:02x}", b);
        }
        s
    }

    fn signed_to_committee_public_key(signed_pk: SignedPublicKey) -> CommitteePublicKey {
        let uncompressed = signed_pk.public_key.inner.serialize_uncompressed(); // [0x04 | X(32) | Y(32)]
        let x = &uncompressed[1..33];
        let y = &uncompressed[33..65];

        CommitteePublicKey {
            x: Self::to_hex_prefixed(x),
            y: Self::to_hex_prefixed(y),
            r: Self::to_hex_prefixed(&signed_pk.signature_r),
            s: Self::to_hex_prefixed(&signed_pk.signature_s),
            v: signed_pk.recovery_id + 27, // Convert to Ethereum's v format (27 or 28)
        }
    }

    fn get_hardcoded_operators_addresses(&self) -> Vec<P2PAddress> {
        // Hardcoded addresses for the 4 BitVMX operators
        vec![
            P2PAddress {
                address: "/ip4/127.0.0.1/tcp/61180".to_string(),
                peer_id: PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100b0595a239c455f955ac2617061fadc0f3c532056da4a4ab4111b6581a62143e6c00b3041a00c290232fa65794ea0a55ca5f2ed3310ecbcab06a721d66e99a27e0d1b8a6afd8e395b741fbcf6cb73294eaeff43118f828f0118a4b5fdc95d472bcadaf2bc4d665e535ccd70b8ee5b82624794351a82c9f819d9a53638122228d1800d7d6561ae98183ae53c6cf23964c7eceeae95807db49a164cfbbc1ddc87a975fbe3d43545e8ce1bad2043cfe6a9aa3a7538ebdab8e6b900c94a691c1321d7c2d7f1a1beb3c3ef03686f7805ce938c92c8d5057cb5101cd51c1d97d7d3d4b9f13b7cb28bc5c4c5c9983a3062efc606b9c440021e1d5257d88d9c3ced0ac38f0203010001".to_string()),
            },
            P2PAddress {
                address: "/ip4/127.0.0.1/tcp/61181".to_string(),
                peer_id: PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100c96872f74e913fbcf2e068d7f508e52dad5a278123ad6546d9735e3f35163e836427ef6ea14ff28d4ca30e7f0d4e251ddf4724668675052d6adb8581550b0adb11f0dcb78a4e9d6ad00f68bf21851d590d88d9fff1d8d7678454f9df4a1daad2f8ebfe69b4ea99160a9e2d43a98cdaaaf380bc4de9f9dec6bedc9351c89c43e4d5d89abbef98664f5d57cdf5c68d93e928203c84fd038fedddac5bbe2b243378141edec442e83c57f0bab437336586f6d6bc01bee222ee8f67dfacb2d94d7a4e406d05446c9f84de055d6175217de19d1005203674b1693f1df2d3dacd11839a782c343c33e86b952740812da624f2ddfd71edf9eb5e9ddf7944b9afc3a08b2f0203010001".to_string()),
            },
            P2PAddress {
                address: "/ip4/127.0.0.1/tcp/61182".to_string(),
                peer_id: PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100e602dadfc9a2b10e6c042e10ba19628e49132fba6197f817457bd8728e881b35dc107838437b562cb9c611c2666fe3492db881630cd917178d17d21d48e664f685d9cd2ea2658501b3eb51ac7d9832e4ec580a5822616b0b663a3fb05a5aae15881baddeb7d8d329f064b460637a28ed569b93074446cb4946720474950456c950b5ae00b5f8b5a490eb1fc9af0206178ab81d3ca81b74fca1d84da9db510c10be2df4624be64fed6a6e59dc90880dc6ed61d4908ddcaf9eb0b08b0d58c5741085da051c4a537d33a8602fc22c6bef5853208698752561afa02ce763fb2bc0b88db51c90735d72dbd0ef6895c77aead64d5fe43e4d7521ed5f8da50c96636e4b0203010001".to_string()),
            },
            P2PAddress {
                address: "/ip4/127.0.0.1/tcp/61183".to_string(),
                peer_id: PeerId("30820122300d06092a864886f70d01010105000382010f003082010a0282010100d1f76c66923556eaa6e9db0acf025fa96049e150cccd910ed6a36d6b32e1eb531620182c34b9ec04a00ba9e2f02f6f6f1493cf0dd42ffcafe60d81c7102f7b64f22a76ebe749dd285435a4d551ed03271062318e08efafbb1e9341aabe685a56cf81abf4af7437e60e9435a0a9682f8720b3ad017c29c517c3b25cc467f5f1ccd9ab791a206cef513141938491e5527df1e615088061a7bdc19622fd43323a74020870042ce33287f730fa5d17eb7f21b1dc6bb028d2a01850b9fb3c0ae40d5023dcdd2c888691a2c50d956f8e6d3d92c3cf893388f954781d1ee118b5840ef88a0d1cc8d218e535d706b044bf6c881ceafec982fd7ed516daaab60c4ea7d15b0203010001".to_string()),
            },
        ]
    }

    fn get_hardcoded_operators_keys(&self) -> Vec<PublicKey> {
        // TODO: Replace with actual public keys obtained from each operator
        // For now, using the current member's key repeated 4 times as placeholder
        let placeholder_key = self.ctx_my_take_key().unwrap_or_else(|_| {
            // Fallback placeholder key if we don't have one yet
            PublicKey::from_slice(&[
                0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x01,
            ])
            .unwrap()
        });

        vec![placeholder_key; 4] // Placeholder - should be actual keys from each operator
    }

    fn get_deterministic_take_key_id(&self) -> Uuid {
        // Hardcoded UUID for take key - same on every run
        Uuid::parse_str("12345678-1234-5678-9abc-123456789abc").unwrap()
    }

    fn get_deterministic_dispute_key_id(&self) -> Uuid {
        // Hardcoded UUID for dispute key - same on every run
        Uuid::parse_str("87654321-4321-8765-cba9-987654321cba").unwrap()
    }

    fn request_take_aggregated_key(&mut self) -> Result<()> {
        let take_key_id = self.get_deterministic_take_key_id();
        self.state.ctx.agg_take_key = Some((take_key_id, None));
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetAggregatedPubkey(take_key_id));
        Ok(())
    }

    fn request_dispute_aggregated_key(&mut self) -> Result<()> {
        let dispute_key_id = self.get_deterministic_dispute_key_id();
        self.state.ctx.agg_dispute_key = Some((dispute_key_id, None));
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetAggregatedPubkey(
            dispute_key_id,
        ));
        Ok(())
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
                // start next
                self.request_bitvmx_comm_info();
            }
            Steps::GetMyCommInfo => {
                self.state.ctx.my_comm_info = Some(data.into_p2p_address()?);
                // start next
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
                // start next
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
                // TODO(iago) process data received from contracts
                self.setup_bitvmx_aggregated_take_pubkey()?;
                // self.request_take_aggregated_key()?;
            }
            Steps::SetupTakeAggregatedKey => {
                Self::close_agg_key_req(
                    &mut self.state.ctx.agg_take_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.setup_bitvmx_aggregated_dispute_pubkey()?;
                // self.request_dispute_aggregated_key()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                Self::close_agg_key_req(
                    &mut self.state.ctx.agg_dispute_key,
                    self.state.flow_id,
                    req_id,
                    data,
                )?;

                self.setup_dispute_core_protocol()?;
            }
            Steps::SetupDisputeCoreProtocol => {
                // TODO(iago) continue flow here
            }
            Steps::Complete => {
                info!("Setup committee flow complete")
            }
        };

        let next_step = self.state.step.next()?;
        self.state.step = next_step;

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
                committee_public_keys: vec![
                    Self::signed_to_committee_public_key(self.ctx_signed_take_key()?),
                    Self::signed_to_committee_public_key(self.ctx_signed_dispute_key()?),
                    Self::signed_to_committee_public_key(self.ctx_signed_comm_key()?),
                ],
                // TODO(iago) fix later
                funding_utxo: UTXO {
                    txid: FixedBytes::default(),
                    outputIndex: 0,
                    amount: 0,
                },
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
        // TODO(ask-Agus) do we need it to be always the same? cant' we generate an uuid and keep it in the flow state?
        let take_key_id = self.get_deterministic_take_key_id();
        self.state.ctx.agg_take_key = Some((take_key_id, None));

        // Use hardcoded take keys from all operators
        let committee_take_keys = self.get_hardcoded_operators_keys();

        // Just setup the key, don't request the aggregated result yet
        let addresses = self.get_hardcoded_operators_addresses();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            take_key_id,
            addresses,
            Some(committee_take_keys),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn setup_bitvmx_aggregated_dispute_pubkey(&mut self) -> Result<()> {
        // TODO(ask-Agus) do we need it to be always the same? cant' we generate an uuid and keep it in the flow state?
        let dispute_key_id = self.get_deterministic_dispute_key_id();
        self.state.ctx.agg_dispute_key = Some((dispute_key_id, None));

        // Use hardcoded dispute keys from all operators
        let committee_dispute_keys = self.get_hardcoded_operators_keys();

        // Just setup the key, don't request the aggregated result yet
        let addresses = self.get_hardcoded_operators_addresses();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
            dispute_key_id,
            addresses,
            Some(committee_dispute_keys),
            NO_LEADER_IDX,
        ));

        Ok(())
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
        info!("Setting up dispute core protocol");

        let committee_id = self.setup_committee()?;

        let members: Vec<Member> = self.get_members_from_contract()?;

        // TODO(ask-Fairgate) is this ok?
        let addresses: Vec<P2PAddress> = members
            .iter()
            .map(|m| P2PAddress {
                address: m.memberAddress.to_lower_hex_string(),
                peer_id: PeerId("what goes here???".to_string()),
            })
            .collect();

        for (idx, member) in members.iter().enumerate() {
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

    fn get_flow_for_stream_id(&self, stream_id: u64) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(ask-Fairgate) implement when clarified how to relate events with flows
        None
    }

    fn get_flow_for_committee_id(
        &self,
        committee_id: U256,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(ask-Fairgate) implement when clarified how to relate events with flows
        None
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
                // TODO(iago) this won't be used by BitVMX, its' just for our logging, make that clear by changing its type maybe
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
                let data = &new_committee_pending.inner;

                // TODO(ask-Fairgate) for some reason streamId is U256 for NewPendingCommittee but u64 for AllCommunicationDataReady
                let stream_id = data.streamId.try_into().with_context(|| {
                    format!(
                        "could not parse streamId {} to u64 for NewCommitteePending event",
                        data.streamId
                    )
                })?;

                // TODO(ask-Fairgate) how to identify the matching flow with the provided data?
                if let Some(first_flow) = self.get_flow_for_stream_id(stream_id) {
                    first_flow.complete_step_and_next(
                        None,
                        StepData::NewCommitteePending(new_committee_pending.clone()),
                    )?
                } else {
                    bail!("No flow found for {new_committee_pending:?}")
                }
            }
            RskPegManagerEvents::NewCommitteeReady(new_committee_ready) => {
                let data = &new_committee_ready.inner;

                // TODO(ask-Fairgate) how to identify the matching flow with the provided data?
                if let Some(first_flow) = self.get_flow_for_committee_id(data.committeeId) {
                    first_flow.complete_step_and_next(
                        None,
                        StepData::NewCommitteeReady(new_committee_ready.clone()),
                    )?
                } else {
                    bail!("No flow found for {new_committee_ready:?}")
                }
            }
            RskPegManagerEvents::AllCommunicationDataReady(all_comm_data_ready) => {
                let data = &all_comm_data_ready.inner;

                if let Some(first_flow) = self.get_flow_for_stream_id(data.streamId) {
                    first_flow.complete_step_and_next(
                        None,
                        StepData::AllCommunicationDataReady(all_comm_data_ready.clone()),
                    )?
                } else {
                    bail!("No flow found for {all_comm_data_ready:?}")
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
            OutgoingBitVMXApiMessages::AggregatedPubkeyNotReady(req_id) => {
                // Key is not ready yet, retry the request
                if let Some(flow) = self.get_flow_for_request_id(req_id) {
                    // Retry the GetAggregatedPubkey request
                    flow.send_bitvmx_msg(IncomingBitVMXApiMessages::GetAggregatedPubkey(*req_id));
                } else {
                    bail!(
                        "No flow found for OutgoingBitVMXApiMessages::AggregatedPubkeyNotReady and id {req_id}"
                    );
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_block(&mut self, _block: &RskBlockAndUncles) -> Result<()> {
        // find flows in status ApplyToStream
        for flow in self.flows.values_mut() {
            if flow.state.step == Steps::ApplyToStream {
                // TODO(iago) properly handle confirmations, now we assume confirmed immediately
                flow.complete_step_and_next(None, StepData::ApplyToStreamConfirmed)?;
            }
        }

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
