use crate::blockchain_tracker::BlockchainView;
use crate::event_processor::EventProcessor;
use crate::types::{
    AllCommunicationDataReadyEvent, MemberOfCommittee, NewCommitteePendingEvent,
    NewCommitteeReadyEvent, RskPegManagerEvents, UserRequests,
};
use alloy_primitives::{Address, FixedBytes};
use anyhow::{Context, Result, bail};
use bitcoin::hashes::Hash;
use bitcoin::key::Parity::Even;
use bitcoin::{Amount, CompressedPublicKey, Network, PublicKey, ScriptBuf, Txid, XOnlyPublicKey};
use bitvmx_bitcoin_rpc;
use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use common::msg_broker::bitvmx_types::{
    IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, OutputType, P2PAddress, PartialUtxo,
    ParticipantRole, PeerId, SignedPublicKey, Utxo,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use log::{debug, error, info};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use tiny_keccak::{Hasher, Keccak};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::user_requests::ApplyToStream;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommitteeMember, CommunicationData, UTXO,
};

use crate::flows::common::build_communication_data;
use crate::flows::dispute_core_setup::DisputeCoreSetup;
use common::types;
use common::types::{CommitteeId, RskBlockAndUncles, StreamId};

use transaction_dispatcher::types::{
    ApplyToStreamInput, CommitteeECDSA, DepositAggregatedKeyInput, DepositCommunicationDataInput,
    DepositCommunicationDataOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    GetMemberPublicKeysOutput, P2PAddressParser,
};

#[cfg(test)]
use mockall::automock;

pub(crate) const NO_LEADER_IDX: u16 = 0;
const TAKE_KEY_INDEX: usize = 0;
const DISPUTE_KEY_INDEX: usize = 1;
const COMM_KEY_INDEX: usize = 2;

// TODO temporary for Regtest stage
const REGTEST: Network = Network::Regtest;

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
type PubKeyReq = Option<(
    Uuid,
    Option<PublicKey>,
    Option<Uuid>,
    Option<SignedPublicKey>,
)>; // request id key, raw pub key, req id signing, signed pub key
type AggKeyReq = Option<(Uuid, Option<PublicKey>)>; // request id, response data
type SetupCoreReq = Option<(Uuid, CommitteeId, Option<String>)>; // request id, committee id, response data // TODO(iago-2) TBC what to store here in data

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
    communication_data_ready: Option<Vec<P2PAddress>>,
    committee_ready: Option<NewCommitteeReadyEvent>,
}

impl FlowContext {
    fn get_stream_id(&self) -> Result<StreamId> {
        Ok(self
            .user_input
            .as_ref()
            .context("Missing stream_id")?
            .stream_id
            .clone()
            .into())
    }

    fn get_committee_id(&self) -> Result<CommitteeId> {
        Ok(self
            .committee_pending
            .as_ref()
            .context("Missing committee pending event")?
            .inner
            .committeeId
            .into())
    }

    fn get_committee_pending_members(&self) -> Result<Vec<CommitteeMember>> {
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

    fn get_committee_ready(&self) -> Result<Committee> {
        let committee = self
            .committee_ready
            .as_ref()
            .context("Missing committee ready event")?
            .inner
            ._committee
            .clone();

        Ok(committee)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steps {
    UserRequest,
    GetMyCommInfo,
    GetMyTakeKey,
    SignMyTakeKey,
    GetMyDisputeKey,
    SignMyDisputeKey,
    GetMyCommKey,
    SignMyCommKey,
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
            Steps::GetMyTakeKey => Steps::SignMyTakeKey,
            Steps::SignMyTakeKey => Steps::GetMyDisputeKey,
            Steps::GetMyDisputeKey => Steps::SignMyDisputeKey,
            Steps::SignMyDisputeKey => Steps::GetMyCommKey,
            Steps::GetMyCommKey => Steps::SignMyCommKey,
            Steps::SignMyCommKey => Steps::ApplyToStream,
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
    PublicKey(PublicKey),
    SignedMessage([u8; 32], [u8; 32], u8), // signature_r, signature_s, recovery_id

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

    fn into_pubkey(self) -> Result<PublicKey> {
        match self {
            StepData::PublicKey(pk) => Ok(pk),
            _ => bail!("Expected PublicKey"),
        }
    }

    fn into_signed_payload(self) -> Result<([u8; 32], [u8; 32], u8)> {
        match self {
            StepData::SignedMessage(r, s, recovery_id) => Ok((r, s, recovery_id)),
            _ => bail!("Expected SignedMessage"),
        }
    }

    fn into_committee_pending(self) -> Result<NewCommitteePendingEvent> {
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

    fn into_committee_ready(self) -> Result<NewCommitteeReadyEvent> {
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
    bitcoin_client: BitcoinClient,
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
            bitcoin_client: Self::build_bitcoin_client_regtest(),
        }
    }

    // this is temporary for Regtest stage, required for utxo generation
    fn build_bitcoin_client_regtest() -> BitcoinClient {
        let config_bitcoin_client = RpcConfig::new(
            REGTEST,
            "http://127.0.0.1:18443".to_string(),
            "foo".to_string(),
            "rpcpassword".to_string(),
            "test_wallet".to_string(),
        );

        let bitcoin_client = BitcoinClient::new_from_config(&config_bitcoin_client)
            .expect("Cannot create Setup Committee Flow without a Bitcoin Client");

        bitcoin_client
    }

    fn my_address(&self) -> types::Address {
        self.contracts.my_address()
    }

    fn request_bitvmx_pub_key_signing(&self, req_id: Uuid, req: &PubKeyReq) -> Result<()> {
        let pub_key = req
            .as_ref()
            .context("Missing Public Key request")?
            .1
            .as_ref()
            .context("Missing Sign Public Key request")?;

        let hash = create_pubkey_hash(pub_key)?;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SignMessage(
            req_id,
            hash.to_vec(),
            *pub_key,
        ));

        Ok(())
    }

    fn close_pub_key_req(
        pub_key_req: &mut PubKeyReq,
        key_req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<Uuid> {
        let key_req_id =
            key_req_id.context("Missing request id on close_pub_key_req".to_string())?;

        match pub_key_req {
            Some(r) if r.0 == key_req_id => {
                let pub_key = data.into_pubkey()?;
                r.1 = Some(pub_key);

                info!(
                    "Got Public Key: {}",
                    hex::encode(pub_key.inner.serialize_uncompressed())
                );

                let sign_req_id = Uuid::new_v4();
                r.2 = Some(sign_req_id);

                Ok(sign_req_id)
            }
            Some(r) => {
                bail!("Request id {key_req_id} does not match expected {r:?}")
            }
            None => {
                bail!("Public Key request missing in context")
            }
        }
    }

    fn close_pub_key_signing_req(
        pub_key_req: &mut PubKeyReq,
        req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<()> {
        let req_id = req_id.context("Missing request id on close_pub_key_signing_req")?;

        let (signature_r, signature_s, recovery_id) = data.into_signed_payload()?;

        match pub_key_req {
            Some(r) if r.2 == Some(req_id) => {
                let public_key = r.1.context("Missing Public Key to sign")?;

                let signed_pubkey =
                    construct_signed_pubkey(public_key, signature_r, signature_s, recovery_id);

                r.3 = Some(signed_pubkey);

                Ok(())
            }
            Some(r) => {
                bail!("Request id {req_id} does not match expected {r:?}")
            }
            None => {
                bail!("Public Key request missing in context")
            }
        }
    }

    fn close_agg_key_req(
        pub_key_req: &mut AggKeyReq,
        req_id: Option<Uuid>,
        data: StepData,
    ) -> Result<()> {
        let req_id = req_id.context("Missing request id on close_agg_key_req".to_string())?;

        match pub_key_req {
            Some(r) if r.0 == req_id => {
                r.1 = Some(data.into_pubkey()?);
                Ok(())
            }
            Some(r) => {
                bail!("Request id {req_id} does not match expected {r:?}")
            }
            None => {
                bail!("Aggregated Key request missing in context")
            }
        }
    }

    fn close_communication_data_step(&mut self) -> Result<()> {
        let my_comm_data = self.build_my_communication_data()?;
        self.state.ctx.communication_data_ready = Some(my_comm_data);
        Ok(())
    }

    fn get_take_aggregated_key_id(&self) -> Result<Uuid> {
        let mut hasher = Sha256::new();

        let committee_id = *self.state.ctx.get_committee_id()?;
        hasher.update(committee_id.to_be_bytes());
        hasher.update("take_aggregated_key");

        // Get the result as a byte array
        let hash = hasher.finalize();
        Uuid::from_slice(&hash[0..16]).context("Failed to convert hash to Uuid")
    }

    fn get_dispute_aggregated_key_id(&self) -> Result<Uuid> {
        let mut hasher = Sha256::new();

        let committee_id = self.state.ctx.get_committee_id()?;
        hasher.update(committee_id.to_be_bytes());
        hasher.update("dispute_aggregated_key");

        // Get the result as a byte array
        let hash = hasher.finalize();
        Uuid::from_slice(&hash[0..16]).context("Failed to convert hash to Uuid")
    }

    fn request_bitvmx_member_pub_key(&self, req_id: Uuid) -> Result<()> {
        Ok(self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetPubKey(req_id, true)))
    }

    // TODO(iago-3) move ctx_xxx methods to FlowContext struct

    // TODO(iago-3) review the ctx_xxx methods we have and try to unify / optimize them

    fn ctx_my_take_key(&self) -> Result<SignedPublicKey> {
        let signed_pubkey = self
            .state
            .ctx
            .my_take_key
            .as_ref()
            .context("Missing request for My Take Key")?
            .3
            .as_ref()
            .context("Missing My Signed Take Key in context")?;

        Ok(signed_pubkey.clone())
    }

    fn ctx_my_dispute_key(&self) -> Result<SignedPublicKey> {
        let signed_pubkey = self
            .state
            .ctx
            .my_dispute_key
            .as_ref()
            .context("My Dispute Key request missing in context")?
            .3
            .as_ref()
            .context("My Signed Dispute Key missing in context")?;

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
        let agg_take_key = self
            .state
            .ctx
            .agg_take_key
            .as_ref()
            .context("Aggregated Take Key request missing in context")?
            .1
            .as_ref()
            .context("Aggregated Take Key missing in context")?;

        Ok(*agg_take_key)
    }

    fn ctx_aggregated_dispute_key(&self) -> Result<PublicKey> {
        let dispute_data_pk = self
            .state
            .ctx
            .agg_dispute_key
            .as_ref()
            .context("Aggregated Dispute Key request missing in context")?
            .1
            .as_ref()
            .context("Aggregated Dispute Key missing in context")?;

        Ok(*dispute_data_pk)
    }

    fn ctx_my_communication_data(&self) -> Result<Vec<P2PAddress>> {
        self.state
            .ctx
            .communication_data_ready
            .as_ref()
            .cloned()
            .context("Missing Communication Data in context")
    }

    fn get_member_keys_by_type(&self, member_addr: Address, key_index: usize) -> Result<PublicKey> {
        let member = self
            .state
            .ctx
            .get_committee_pending_members()?
            .into_iter()
            .find(|m| m.memberAddress == member_addr)
            .with_context(|| format!("Member {member_addr} not found in committee members"))?;

        self.get_member_key(key_index, member)
    }

    fn get_member_key(&self, key_index: usize, member: CommitteeMember) -> Result<PublicKey> {
        let key_type = match key_index {
            TAKE_KEY_INDEX => "Take",
            DISPUTE_KEY_INDEX => "Dispute",
            _ => bail!("Invalid key index: {key_index}, expected 0 (take) or 1 (dispute)"),
        };

        let member_addr = member.memberAddress;
        let keys = self.get_member_public_keys_from_contracts(member_addr)?;
        let key_str = keys
            .public_keys
            .get(key_index)
            .with_context(|| format!("{key_type} Key not found on Committee for {member_addr}"))?;

        // TODO revisit this, we are encoding bytes to hex string in the contracts to then decode it back to bytes here

        let key_bytes: FixedBytes<32> = key_str
            .parse()
            .context("Failed to parse public key str to FixedBytes<32>")?;
        let x_only_key = XOnlyPublicKey::from_slice(key_bytes.as_slice())
            .context("Failed to parse aggregated public key")?;

        debug!("Got {key_type} Key for member {member_addr} with X: {x_only_key:?}");

        // BitVMX adjusts parity to Even, so we do the same here
        let secp_key = x_only_key.public_key(Even);
        let member_key = PublicKey::new(secp_key);

        Ok(member_key)
    }

    fn build_member_funding_utxo(
        &self,
        member_dispute_key: &PublicKey,
        utxo: &UTXO,
    ) -> Result<PartialUtxo> {
        let tx_id = Txid::from_slice(utxo.txid.as_slice())
            .context("Could not get Bitcoin TxId from contracts utxo")?;

        let script_pubkey = ScriptBuf::new_p2wpkh(
            &member_dispute_key
                .wpubkey_hash()
                .context("Failed to get wpubkey_hash from dispute public key")?,
        );

        let output_type = OutputType::SegwitPublicKey {
            value: Amount::from_sat(utxo.amount),
            script_pubkey,
            public_key: *member_dispute_key,
        };

        Ok((
            tx_id,
            utxo.outputIndex,
            Some(utxo.amount),
            Some(output_type),
        ))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        info!("Sending {msg:?} to BitVMX");

        let result = self.bitvmx_broker.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {:?}", result);
        }
    }

    fn ctx_user_input(&self) -> Result<ApplyToStream> {
        self.state
            .ctx
            .user_input
            .as_ref()
            .context("Missing User Input in context")
            .map(|input| input.clone())
    }

    fn get_member_public_keys_from_contracts(
        &self,
        member_address: Address,
    ) -> Result<GetMemberPublicKeysOutput> {
        self.rt_sync.run(
            self.contracts
                .get_member_public_keys(GetMemberPublicKeysInput { member_address }),
        )
    }

    // this is temporary for Regtest stage, required for utxo generation
    fn generate_my_utxo_regtest(&self, utxo_val: u64) -> Result<(Txid, u32)> {
        let pub_key = self.ctx_my_dispute_key()?.public_key;
        let compressed =
            CompressedPublicKey::try_from(pub_key).context("Failed to compress public key")?;
        let funding_wallet = bitcoin::Address::p2wpkh(&compressed, REGTEST);

        let fund_res = &self
            .bitcoin_client
            .fund_address(&funding_wallet, Amount::from_sat(utxo_val))
            .context("Failed to fund address on fake utxo generation")?;

        debug!("Generated regtest UTXO: {:?}", fund_res);

        let utxo_tx_id = fund_res.0.compute_txid();
        let output = fund_res.1;

        Ok((utxo_tx_id, output))
    }

    fn build_my_communication_data(&self) -> Result<Vec<P2PAddress>> {
        let committee_id = self
            .state
            .ctx
            .get_committee_id()
            .context("Get Communication Data")?;

        let my_address: Address = self.my_address().into();
        let input = GetCommunicationDataInput {
            // TODO rethink if this is needed or a member should only request its own communication data and therefore this param is not required
            member_address: my_address,
            committee_id,
        };

        let comm_data = self
            .rt_sync
            .run(self.contracts.get_committee_communication_data(input))?;

        let committee_addresses = comm_data
            .communication_data
            .into_iter()
            .map(|data| P2PAddressParser::addr_from_contracts(&data))
            .collect::<Result<Vec<_>>>()?;

        let my_p2p_address = self.ctx_my_comm_info()?.address;

        // temporarily stored PeerId as the communication key, agreed with Fairgate
        let committee_peer_ids = self.get_committee_peer_ids()?;

        build_communication_data(my_p2p_address, committee_addresses, committee_peer_ids)
    }

    fn deposit_communication_data(&self) -> Result<DepositCommunicationDataOutput> {
        let committee_id = self
            .state
            .ctx
            .get_committee_id()
            .context("Deposit Communication Data")?;

        let my_p2p_address = self.ctx_my_comm_info()?;

        let mut communication_data = vec![];
        // communication data size
        for member in self.state.ctx.get_committee_pending_members()? {
            let my_address: Address = self.my_address().into();
            if member.memberAddress == my_address {
                // contracts require zeroed communication data for my own address on deposit
                communication_data.push(CommunicationData::default())
            } else {
                let data = P2PAddressParser::addr_to_contracts(&my_p2p_address.address)?;
                communication_data.push(data);
            }
        }

        info!(
            "Depositing member {} communication data for stream {}: {communication_data:?}",
            self.my_address(),
            *committee_id
        );

        self.rt_sync.run(
            self.contracts
                .deposit_communication_data(DepositCommunicationDataInput {
                    committee_id,
                    communication_data,
                }),
        )
    }

    fn deposit_aggregated_key(&self) -> Result<()> {
        let aggregated_take_key = self
            .ctx_aggregated_take_key()
            .context("Deposit Aggregated Key")?;

        let committee_id = self.state.ctx.get_committee_id()?;

        info!(
            "Depositing aggregated key for stream {}: {}",
            aggregated_take_key.to_string(),
            *committee_id
        );

        let x_only_key = XOnlyPublicKey::from(aggregated_take_key);
        let aggregated_key = FixedBytes::<32>::try_from(&x_only_key.serialize())
            .context("Failed to serialize aggregated public key")?;

        let input = DepositAggregatedKeyInput {
            committee_id,
            aggregated_key,
        };

        self.rt_sync
            .run(self.contracts.deposit_aggregated_key(input))?;

        Ok(())
    }

    fn get_committee_keys_by_type(&self, key_index: usize) -> Result<Vec<PublicKey>> {
        let mut committee_pub_keys = vec![];

        for member in self.state.ctx.get_committee_pending_members()? {
            let member_key = self.get_member_key(key_index, member)?;
            committee_pub_keys.push(member_key);
        }

        Ok(committee_pub_keys)
    }

    fn get_committee_peer_ids(&self) -> Result<Vec<PeerId>> {
        let mut peer_ids = vec![];

        for member in self.state.ctx.get_committee_pending_members()? {
            let member_addr = member.memberAddress;
            let keys = self.get_member_public_keys_from_contracts(member_addr)?;
            let key_str = keys.public_keys.get(COMM_KEY_INDEX).with_context(|| {
                format!("Communication key not found on Committee for {member_addr}")
            })?;

            debug!("Member {member_addr} PeerId: {key_str:?}");

            // key_str already decoded
            peer_ids.push(PeerId(key_str.to_string()));
        }

        Ok(peer_ids)
    }

    fn build_members_of_committee(
        &mut self,
        committee: Committee,
    ) -> Result<Vec<MemberOfCommittee>> {
        let mut member_of_committee = vec![];

        // TODO(iago-3) rethink how we store the committee member data in the context, we can unify it in a MemberOfCommittee struct and reduce the number of ctx_xxx methods
        for (idx, cm) in committee.members.iter().enumerate() {
            debug!("Processing committee member {idx:?} {cm:?}");

            // TODO(iago-3) move it to a From trait impl
            let role = if cm.role == 1 {
                ParticipantRole::Prover
            } else if cm.role == 2 {
                ParticipantRole::Verifier
            } else {
                bail!("Invalid member role: {}", cm.role);
            };

            // TODO mini optimization: do not request my data, it is in context already

            let take_key = self.get_member_keys_by_type(cm.memberAddress.into(), TAKE_KEY_INDEX)?;
            let dispute_key =
                self.get_member_keys_by_type(cm.memberAddress.into(), DISPUTE_KEY_INDEX)?;

            let contracts_utxo = committee
                .fundingUTXOs
                .get(idx)
                .context("Missing utxo for committee member")?;

            let funding_utxo = self.build_member_funding_utxo(&dispute_key, contracts_utxo)?;

            let moc = MemberOfCommittee {
                address: cm.memberAddress.into(),
                role,
                take_key,
                dispute_key,
                funding_utxo,
            };

            member_of_committee.push(moc);
        }
        Ok(member_of_committee)
    }
}

impl<CG, BC> SetupCommitteeFlowApi for SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn complete_step_and_next(&mut self, req_id: Option<Uuid>, data: StepData) -> Result<()> {
        let current_step = self.state.step;

        info!(
            "Completing step {current_step:?} for flow {} with req_id {req_id:?} and data {data:?}",
            self.state.flow_id
        );
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
                let sign_req_id =
                    Self::close_pub_key_req(&mut self.state.ctx.my_take_key, req_id, data)?;
                self.request_bitvmx_pub_key_signing(sign_req_id, &self.state.ctx.my_take_key)?;
            }
            Steps::SignMyTakeKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_take_key, req_id, data)?;
                self.request_bitvmx_dispute_pub_key()?;
            }
            Steps::GetMyDisputeKey => {
                let sign_req_id =
                    Self::close_pub_key_req(&mut self.state.ctx.my_dispute_key, req_id, data)?;
                self.request_bitvmx_pub_key_signing(sign_req_id, &self.state.ctx.my_dispute_key)?;
            }
            Steps::SignMyDisputeKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_dispute_key, req_id, data)?;
                self.request_bitvmx_comm_pub_key()?;
            }
            Steps::GetMyCommKey => {
                let sign_req_id =
                    Self::close_pub_key_req(&mut self.state.ctx.my_comm_key, req_id, data)?;
                self.request_bitvmx_pub_key_signing(sign_req_id, &self.state.ctx.my_comm_key)?;
            }
            Steps::SignMyCommKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_comm_key, req_id, data)?;
                self.apply_to_stream()?;
            }
            Steps::ApplyToStream => {
                // TODO(iago-2) sometimes it gets stuck in "successful apply to stream", investigate why

                let pending_committee = data.into_committee_pending()?;

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
                Self::close_agg_key_req(&mut self.state.ctx.agg_take_key, req_id, data)?;

                self.setup_bitvmx_aggregated_dispute_pubkey()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                Self::close_agg_key_req(&mut self.state.ctx.agg_dispute_key, req_id, data)?;

                self.deposit_aggregated_key()?;
            }
            Steps::DepositAggregatedKey => {
                self.state.ctx.committee_ready = Some(data.into_committee_ready()?);

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
        self.state.ctx.my_take_key = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_dispute_key = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_comm_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_comm_key = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn apply_to_stream(&self) -> Result<()> {
        let user_input = self.ctx_user_input()?;

        let utxo_val = user_input.funding_utxo.value;
        let (tx_id, output) = self
            .generate_my_utxo_regtest(utxo_val)
            .context("Generating funding UTXO")?;

        let utxo = UTXO {
            txid: FixedBytes::from(tx_id.to_byte_array()),
            outputIndex: output,
            amount: utxo_val,
        };

        let stream_id = self.state.ctx.get_stream_id()?;

        let input = ApplyToStreamInput {
            stream_id: stream_id.clone(),
            role: u8::from(user_input.role),
            take_key: signed_to_committee_public_key(self.ctx_my_take_key()?),
            dispute_key: signed_to_committee_public_key(self.ctx_my_dispute_key()?),
            peer_id: self.ctx_my_comm_info()?.peer_id,
            funding_utxo: utxo,
        };

        debug!("Applying to stream with {input:?}");

        let res = self.rt_sync.run(self.contracts.apply_to_stream(input));

        if res.is_err() {
            bail!("Failed to apply to stream: {:?}", res);
        }

        Ok(())
    }

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()> {
        info!("Setup BitVMX Aggregated Take key");

        let take_key_id = self.get_take_aggregated_key_id()?;
        self.state.ctx.agg_take_key = Some((take_key_id, None));

        let committee_take_keys = self.get_committee_keys_by_type(TAKE_KEY_INDEX)?;
        let communication_data = self.ctx_my_communication_data()?;

        // Bitvmx responds with the aggregated key
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

        let dispute_key_id = self.get_dispute_aggregated_key_id()?;
        self.state.ctx.agg_dispute_key = Some((dispute_key_id, None));

        let committee_dispute_keys = self.get_committee_keys_by_type(DISPUTE_KEY_INDEX)?;
        let communication_data = self.ctx_my_communication_data()?;

        // Bitvmx responds with the aggregated key
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

        let committee = self.state.ctx.get_committee_ready()?;
        let members = self.build_members_of_committee(committee)?;

        let dispute_core = DisputeCoreSetup::new(self.bitvmx_broker.clone());

        let utxo_val = self.ctx_user_input()?.speed_up_utxo.value;
        let (tx_id, output) = self
            .generate_my_utxo_regtest(utxo_val)
            .context("Generating speedup UTXO")?;

        let my_speedup_utxo = Utxo {
            txid: tx_id,
            vout: output,
            amount: utxo_val,
            pub_key: self.ctx_my_dispute_key()?.public_key,
        };

        let p2p_addrs = self.ctx_my_communication_data()?;

        dispute_core.setup(
            self.state.ctx.get_committee_id()?,
            members,
            p2p_addrs,
            self.ctx_aggregated_take_key()?,
            self.ctx_aggregated_dispute_key()?,
            my_speedup_utxo,
        )
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

    fn get_flow_for_stream_id(
        &mut self,
        stream_id: StreamId,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(iago-3) optimize this search by keeping convenient map of stream_id -> flow_id or alike

        // TODO(iago-2) can multiple flows exist for the same stream_id?

        self.flows.values_mut().find(|f| {
            f.state
                .ctx
                .get_stream_id()
                .map_or(false, |id| id == stream_id)
        })
    }

    fn get_flow_for_committee_id(
        &mut self,
        committee_id: CommitteeId,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(iago-3) optimize this search by keeping convenient map of committee_id -> flow_id or alike

        // TODO(iago-2) we have an issue here if multiple flows are created for the same committee_id

        self.flows.values_mut().find(|f| {
            f.state
                .ctx
                .committee_pending
                .as_ref()
                .map_or(false, |ev| ev.inner.committeeId == *committee_id)
        })
    }

    fn get_flow_for_request_id(&mut self, uuid: &Uuid) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(iago-3) super naive approach implemented here for now: find within the different flows and their step datas one with the req_id
        // an alternative could be storing all the requests (ids) for which the flow is waiting response
        // in a same array - but I find this super risky, as it will only work if a) we NEVER send 2
        // "concurrent request-id-depending" messages to BitVMX and b) BitVMX guarantees order in request/response;
        // in addition to that, any change in the code could break it and end up mixing requests/responses/steps

        self.flows.values_mut().find(|flow| {
            if let Some((pk_req_id, _, sign_req_id, _)) = &flow.state.ctx.my_take_key {
                if pk_req_id == uuid || sign_req_id.map_or(false, |id| id == *uuid) {
                    return true;
                }
            }
            if let Some((pk_req_id, _, sign_req_id, _)) = &flow.state.ctx.my_dispute_key {
                if pk_req_id == uuid || sign_req_id.map_or(false, |id| id == *uuid) {
                    return true;
                }
            }
            if let Some((pk_req_id, _, sign_req_id, _)) = &flow.state.ctx.my_comm_key {
                if pk_req_id == uuid || sign_req_id.map_or(false, |id| id == *uuid) {
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

                let stream_id = new_committee_pending.inner._committee.streamId;
                if let Some(first_flow) = self.get_flow_for_stream_id(stream_id.into()) {
                    first_flow.complete_step_and_next(
                        None,
                        StepData::PendingCommittee(new_committee_pending.clone()),
                    )?
                } else {
                    bail!("No flow found for stream {stream_id}")
                }
            }
            RskPegManagerEvents::AllCommunicationDataReady(all_comm_data_ready) => {
                info!(
                    "Processing AllCommunicationDataReady event: {:?}",
                    all_comm_data_ready
                );

                let committee_id = all_comm_data_ready.inner._committeeId.into();
                if let Some(first_flow) = self.get_flow_for_committee_id(committee_id) {
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

                let committee_id = new_committee_ready.inner.committeeId.into();
                if let Some(first_flow) = self.get_flow_for_committee_id(committee_id) {
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
            OutgoingBitVMXApiMessages::PubKey(req_id, public_key) => {
                // Handle PubKey response for GetKey steps
                if let Some(flow) = self.get_flow_for_request_id(req_id) {
                    flow.complete_step_and_next(Some(*req_id), StepData::PublicKey(*public_key))?;
                } else {
                    bail!("No flow found for OutgoingBitVMXApiMessages::PubKey and id {req_id}");
                }
            }
            OutgoingBitVMXApiMessages::SignedMessage(
                sign_req_id,
                signature_r,
                signature_s,
                recovery_id,
            ) => {
                // Handle SignedMessage response using the standard flow
                if let Some(flow) = self.get_flow_for_request_id(sign_req_id) {
                    flow.complete_step_and_next(
                        Some(*sign_req_id),
                        StepData::SignedMessage(*signature_r, *signature_s, *recovery_id),
                    )?;
                } else {
                    bail!(
                        "No flow found for OutgoingBitVMXApiMessages::SignedMessage and id {sign_req_id}"
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

fn signed_to_committee_public_key(signed_pk: SignedPublicKey) -> CommitteeECDSA {
    // TODO(iago-2) this can panic, handle gracefully

    let uncompressed = signed_pk.public_key.inner.serialize_uncompressed(); // [0x04 | X(32) | Y(32)]
    let x = &uncompressed[1..33];
    let y = &uncompressed[33..65];

    CommitteeECDSA {
        x: hex::encode(x),
        y: hex::encode(y),
        r: hex::encode(&signed_pk.signature_r),
        s: hex::encode(&signed_pk.signature_s),
        v: signed_pk.recovery_id + 27, // Convert to Ethereum's v format (27 or 28)
    }
}

// Helper function to create keccak256 hash of uncompressed public key
fn create_pubkey_hash(public_key: &PublicKey) -> Result<[u8; 32]> {
    // Get uncompressed public key coordinates
    let mut pk = *public_key;
    pk.compressed = false;
    let uncompressed_pub_key = pk.to_bytes().split_off(1); // Remove the 0x04 prefix

    // Create keccak256 hash of the uncompressed public key
    let mut keccak = Keccak::v256();
    let mut pub_key_hash = [0u8; 32];
    keccak.update(&uncompressed_pub_key);
    keccak.finalize(&mut pub_key_hash);

    Ok(pub_key_hash)
}

// Helper function to construct SignedPublicKey from components
fn construct_signed_pubkey(
    public_key: PublicKey,
    signature_r: [u8; 32],
    signature_s: [u8; 32],
    recovery_id: u8,
) -> SignedPublicKey {
    SignedPublicKey {
        public_key,
        signature_r,
        signature_s,
        recovery_id,
    }
}
