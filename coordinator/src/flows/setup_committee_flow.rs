use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::UserRequests;
use anyhow::{Context, Result, bail};
use bitcoin::PublicKey;
use bitcoin::hex::DisplayHex;
use common::runtime_sync::RuntimeSync;
use log::{error, info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, NewCommittee, OutgoingBitVMXApiMessages, P2PAddress, PartialUtxo,
    ParticipantRole, PeerId, VariableTypes, SignedPublicKey,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};

use crate::user_requests::ApplyToStream;
use union_contracts::bindings::committee_registry::CommitteeRegistry::CommitteeMember;

#[cfg(test)]
use mockall::automock;
use serde_json::Value;
use transaction_dispatcher::types::{ApplyToStreamInput, CommitteePublicKey};

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
type SetupCoreReq = Option<(Uuid, Uuid, Option<String>)>; // request id, committee id, response data // TODO(iago) TBC what to store here in data

#[derive(Default, Debug)]
struct FlowContext {
    user_input: Option<ApplyToStream>,
    my_comm_info: Option<P2PAddress>,
    my_take_key: PubKeyReq,
    my_dispute_key: PubKeyReq,
    my_comm_key: PubKeyReq,
    agg_take_key: PubKeyReq,
    agg_dispute_key: PubKeyReq,
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

enum StepData {
    UserRequest(ApplyToStream),
    CommInfo(P2PAddress),
    SignedPublicKey(SignedPublicKey),
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
        flow_id: Uuid,
        req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<Option<(Uuid, Option<SignedPublicKey>)>> {
        let req_id = req_id
            .with_context(|| format!("Missing request id for GetMyTakeKey and flow {flow_id}",))?;

        Ok(Some((req_id, Some(data.into_signed_pubkey()?))))
    }

    fn add_my_comm_data(
        &self,
        communication_data: &mut HashMap<PublicKey, P2PAddress>,
    ) -> Result<()> {
        // TODO(Fairgate) awaiting Fairgate to add it to the API
        let my_comm_pubkey = PublicKey::from_slice(&[])?;

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
        // TODO add other members data
        Ok(())
    }

    fn setup_aggregated_pubkey(&self, keys: Vec<PublicKey>) -> Result<()> {
        // TODO(Fairgate) confirm with Fairgate how to get this, if it's from the pending committee or how
        let addresses = vec![];

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

        let signed_pubkey = take_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for Aggregated Take key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;
        
        Ok(signed_pubkey.public_key)
    }

    fn ctx_aggregated_dispute_key(&self) -> Result<PublicKey> {
        let dispute_data = self.state.ctx.agg_dispute_key.as_ref().with_context(|| {
            format!(
                "Missing request for Aggregated Dispute key for flow {}",
                self.state.flow_id
            )
        })?;

        let req_id = dispute_data.0;

        let signed_pubkey = dispute_data.1.as_ref().with_context(|| {
            format!(
                "Missing response for Aggregated Dispute key for flow {} and req_id {req_id}",
                self.state.flow_id
            )
        })?;
        
        Ok(signed_pubkey.public_key)
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
}

impl<CG, BC> SetupCommitteeFlowApi for SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn complete_step_and_next(&mut self, req_id: Option<Uuid>, data: StepData) -> Result<()> {
        let current_step = self.state.step;

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
                self.state.ctx.my_take_key =
                    Self::close_pub_key_req(self.state.flow_id, req_id, data)?;
                self.request_bitvmx_dispute_pub_key()?;
            }
            Steps::GetDisputeKey => {
                self.state.ctx.my_dispute_key =
                    Self::close_pub_key_req(self.state.flow_id, req_id, data)?;
                // start next
                self.request_bitvmx_comm_pub_key()?;
            }
            Steps::GetMyCommKey => {
                self.state.ctx.my_comm_key =
                    Self::close_pub_key_req(self.state.flow_id, req_id, data)?;
                // start next
                self.apply_to_stream()?;
            }
            Steps::ApplyToStream => {
                // TODO(iago) process data received from contracts
                self.setup_bitvmx_aggregated_take_pubkey()?;
            }
            Steps::SetupTakeAggregatedKey => {
                self.state.ctx.agg_take_key =
                    Self::close_pub_key_req(self.state.flow_id, req_id, data)?;
                // start next
                self.setup_bitvmx_aggregated_dispute_pubkey()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                self.state.ctx.agg_dispute_key =
                    Self::close_pub_key_req(self.state.flow_id, req_id, data)?;
                // start next
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
                public_keys: vec![
                    Self::signed_to_committee_public_key(self.ctx_signed_take_key()?),
                    Self::signed_to_committee_public_key(self.ctx_signed_dispute_key()?),
                    Self::signed_to_committee_public_key(self.ctx_signed_comm_key()?),
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
        let mut committee_take_keys = vec![]; // TODO(iago) contracts getMembersPublicKeys and filter for take
        self.state.ctx.agg_take_key = Some((Uuid::new_v4(), None));
        committee_take_keys.push(self.ctx_my_take_key()?);
        self.setup_aggregated_pubkey(committee_take_keys)
    }

    fn setup_bitvmx_aggregated_dispute_pubkey(&mut self) -> Result<()> {
        let mut committee_dispute_keys = vec![]; // TODO(iago) contracts getMembersPublicKeys and filter for dispute
        self.state.ctx.agg_dispute_key = Some((Uuid::new_v4(), None));
        committee_dispute_keys.push(self.ctx_my_dispute_key()?);
        self.setup_aggregated_pubkey(committee_dispute_keys)
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
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

    fn get_flow_for_request_id(&mut self, uuid: &Uuid) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(iago) super naive approach implemented here for now: find within the different flows and their step datas one with the req_id
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
                    flow.complete_step_and_next(Some(*req_id), StepData::SignedPublicKey(signed_key.clone()))?;
                } else {
                    bail!("No flow found for OutgoingBitVMXApiMessages::SignedPubKey and id {req_id}");
                }
            }
            _ => {}
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
