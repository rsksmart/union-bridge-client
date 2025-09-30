use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::event_processor::EventProcessor;
use crate::types::{
    AllCommunicationDataReadyEvent, EventStatus, MemberOfCommittee, NewCommitteePendingEvent,
    NewCommitteeReadyEvent, RskPegManagerEvents, UserRequests,
};
use alloy_primitives::{Address, Bytes, FixedBytes};
use anyhow::{Context, Result, bail, ensure};
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
use log::{debug, error, info, trace, warn};
use sha2::{Digest, Sha256};
use std::any::type_name_of_val;
use std::collections::HashMap;
use std::rc::Rc;
use tiny_keccak::{Hasher, Keccak};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

use crate::user_requests::ApplyToStream;
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommitteeMember, CommunicationData, UTXO,
};

use crate::flows::committee::dispute_core_setup::DisputeCoreSetup;
use crate::flows::common::{
    COMM_KEY_INDEX, DISPUTE_KEY_INDEX, GlobalContext, TAKE_KEY_INDEX, build_communication_data,
};
use common::types;
use common::types::{BlockNumber, CommitteeId, RskBlockAndUncles, StreamId, TxIdParser};

use transaction_dispatcher::types::{
    ApplyToStreamInput, CommitteeECDSA, DepositAggregatedKeyInput, DepositCommunicationDataInput,
    DepositCommunicationDataOutput, GetCommunicationDataInput, GetMemberPublicKeysInput,
    GetMemberPublicKeysOutput, P2PAddressParser,
};

use crate::config::REQUIRED_CONFIRMATIONS;

#[cfg(test)]
use mockall::automock;

pub(crate) const NO_LEADER_IDX: u16 = 0;

// TODO temporary for Regtest stage
const REGTEST: Network = Network::Regtest;

#[cfg_attr(test, automock)]
trait SetupCommitteeFlowApi {
    fn start_step(&mut self, next_step: Steps) -> Result<()>;

    fn complete_step(&mut self, data: StepData) -> Result<()>;

    fn request_bitvmx_comm_info(&self);

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()>;

    fn request_bitvmx_take_pub_key_signing(&mut self) -> Result<()>;

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()>;

    fn request_bitvmx_dispute_pub_key_signing(&mut self) -> Result<()>;

    fn request_bitvmx_comm_pub_key(&mut self) -> Result<()>;

    fn request_bitvmx_comm_pub_key_signing(&mut self) -> Result<()>;

    fn apply_to_stream(&self) -> Result<()>;

    fn deposit_communication_data(&self) -> Result<DepositCommunicationDataOutput>;

    fn update_my_committees(
        &mut self,
        pending_committee: NewCommitteePendingEvent,
        committee_id: &CommitteeId,
    ) -> Result<()>;

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()>;

    fn setup_bitvmx_aggregated_dispute_pubkey(&mut self) -> Result<()>;

    fn deposit_aggregated_key(&self) -> Result<()>;

    fn setup_dispute_core_protocol(&mut self) -> Result<()>;
}

#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowFactoryApi<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi>
{
    fn create_flow(&self, internal_id: Uuid) -> SetupCommitteeFlow<CG, BC>;
}

// TODO improve with structs instead of tuples, using tuples for now for validation
type PubKeyReq = Option<(
    Uuid,
    Option<PublicKey>,
    Option<Uuid>,
    Option<SignedPublicKey>,
)>; // request id key, raw pub key, req id signing, signed pub key
type AggKeyReq = Option<(Uuid, Option<PublicKey>)>; // request id, response data
type SetupCoreReq = Vec<(Uuid, CommitteeId, bool)>; // request id, committee id, response data

#[derive(Default, Debug)]
struct FlowContext {
    // stepped
    user_input: Option<ApplyToStream>,
    my_comm_info: Option<P2PAddress>,
    my_take_key_req: PubKeyReq,
    my_dispute_key_req: PubKeyReq,
    my_comm_key_req: PubKeyReq,
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
    Init,
    GetMyCommInfo,
    GetMyTakeKey,
    SignMyTakeKey,
    GetMyDisputeKey,
    SignMyDisputeKey,
    GetMyCommKey,
    SignMyCommKey,
    ApplyToStream,
    DepositP2PData,
    SetupTakeAggregatedKey,
    SetupDisputeAggregatedKey,
    DepositAggregatedKey,
    SetupDisputeCore,
    Done,
}

#[derive(Debug, Clone)]
enum StepData {
    // sync or member-dependent steps
    UserRequest(ApplyToStream),
    CommInfo(P2PAddress),
    PublicKey(PublicKey),
    SignedMessage([u8; 32], [u8; 32], u8), // signature_r, signature_s, recovery_id
    SetupCompleted(Uuid),

    // async or collaborative steps
    PendingCommittee(NewCommitteePendingEvent),
    ReadyCommunicationData(AllCommunicationDataReadyEvent),
    ReadyCommittee(NewCommitteeReadyEvent),
}

impl StepData {
    fn into_user_input(self) -> Result<ApplyToStream> {
        match self {
            StepData::UserRequest(input) => Ok(input),
            _ => bail!("Expected UserRequest data"),
        }
    }

    fn into_p2p_address(self) -> Result<P2PAddress> {
        match self {
            StepData::CommInfo(addr) => Ok(addr),
            _ => bail!("Expected P2PAddress data"),
        }
    }

    fn into_pubkey(self) -> Result<PublicKey> {
        match self {
            StepData::PublicKey(pk) => Ok(pk),
            _ => bail!("Expected PublicKey data"),
        }
    }

    fn into_signed_payload(self) -> Result<([u8; 32], [u8; 32], u8)> {
        match self {
            StepData::SignedMessage(r, s, recovery_id) => Ok((r, s, recovery_id)),
            _ => bail!("Expected SignedMessage data"),
        }
    }

    fn into_committee_pending(self) -> Result<NewCommitteePendingEvent> {
        match self {
            StepData::PendingCommittee(ev) => Ok(ev),
            _ => bail!("Expected PendingCommittee data"),
        }
    }

    fn into_all_comm_data_ready(self) -> Result<AllCommunicationDataReadyEvent> {
        match self {
            StepData::ReadyCommunicationData(ev) => Ok(ev),
            _ => bail!("Expected ReadyCommunicationData data"),
        }
    }

    fn into_committee_ready(self) -> Result<NewCommitteeReadyEvent> {
        match self {
            StepData::ReadyCommittee(ev) => Ok(ev),
            _ => bail!("Expected ReadyCommittee data"),
        }
    }

    fn into_setup_completed(self) -> Result<Uuid> {
        match self {
            StepData::SetupCompleted(ev) => Ok(ev),
            _ => bail!("Expected SetupCompleted data"),
        }
    }
}

pub(crate) struct State {
    internal_id: Uuid,
    step: Steps,
    ctx: FlowContext,
}

pub(crate) struct SetupCommitteeFlow<CG: RskContractsGatewayApi, BC: BitVmxBrokerClientApi> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    state: State,
    bitcoin_client: BitcoinClient,
    global_context: GlobalContext,
}

impl<CG, BC> SetupCommitteeFlow<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        internal_id: Uuid,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State {
                internal_id,
                step: Steps::Init,
                ctx: FlowContext::default(),
            },
            bitcoin_client: Self::build_bitcoin_client_regtest(),
            global_context,
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

    fn request_bitvmx_key_signing(pub_key_req: &mut PubKeyReq, bitvmx_broker: &BC) -> Result<()> {
        let pub_key_req = pub_key_req.as_mut().context("Missing Public Key request")?;

        let pub_key = pub_key_req
            .1
            .as_ref()
            .context("Missing Sign Public Key request")?;

        let hash = create_pubkey_hash(pub_key)?;

        let req_id = Uuid::new_v4();
        pub_key_req.2 = Some(req_id);

        let result = bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SignMessage(req_id, hash.to_vec(), *pub_key),
        );

        if result.is_err() {
            error!("Failed to send msg to BitVMX: {:?}", result);
        }

        Ok(())
    }

    fn close_pub_key_req(pub_key_req: &mut PubKeyReq, data: StepData) -> Result<()> {
        match pub_key_req {
            Some(r) => {
                let pub_key = data.into_pubkey()?;
                r.1 = Some(pub_key);

                debug!("Got public key for signing");
                trace!("Key: {}", hex::encode(pub_key.inner.serialize_uncompressed()));

                Ok(())
            }
            None => {
                bail!("Public Key request missing in context")
            }
        }
    }

    fn close_pub_key_signing_req(pub_key_req: &mut PubKeyReq, data: StepData) -> Result<()> {
        let (signature_r, signature_s, recovery_id) = data.into_signed_payload()?;

        match pub_key_req {
            Some(r) => {
                let public_key = r.1.context("Missing Public Key to sign")?;

                let signed_pubkey =
                    construct_signed_pubkey(public_key, signature_r, signature_s, recovery_id);

                r.3 = Some(signed_pubkey);

                Ok(())
            }
            None => {
                bail!("Public Key request missing in context")
            }
        }
    }

    fn close_agg_key_req(pub_key_req: &mut AggKeyReq, data: StepData) -> Result<()> {
        match pub_key_req {
            Some(r) => {
                r.1 = Some(data.into_pubkey()?);
                Ok(())
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

    fn close_setup_core_req(setup_core_req: &mut SetupCoreReq, data: StepData) -> Result<bool> {
        let recv_protocol_id = data.into_setup_completed()?;

        for setup_core in setup_core_req.iter_mut() {
            if setup_core.0 == recv_protocol_id {
                setup_core.2 = true; // mark as completed
            }
        }

        let missing_responses = setup_core_req.iter().any(|r| !r.2); // false == not completed

        Ok(missing_responses)
    }

    fn im_selected_to_new_committee(
        &mut self,
        pending_committee: &NewCommitteePendingEvent,
        committee_id: &CommitteeId,
    ) -> Result<bool> {
        if self.global_context.my_committees().im_member(&committee_id) {
            bail!("Already part of committee {committee_id}");
        }

        let was_selected = pending_committee.inner._committee.members.iter().any(|m| {
            let member_addr: types::Address = m.memberAddress.into();
            member_addr == self.my_address()
        });

        Ok(was_selected)
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

    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: move ctx_xxx methods to FlowContext struct

    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: review the ctx_xxx methods we have and try to unify / optimize them

    fn ctx_my_take_key(&self) -> Result<SignedPublicKey> {
        let signed_pubkey = match self.global_context.my_keys().take_key() {
            Some(key) => key,
            None => self
                .state
                .ctx
                .my_take_key_req
                .as_ref()
                .context("Missing request for My Take Key")?
                .3
                .as_ref()
                .context("Missing My Signed Take Key in context")?
                .clone(),
        };

        Ok(signed_pubkey)
    }

    fn ctx_my_dispute_key(&self) -> Result<SignedPublicKey> {
        let signed_pubkey = match self.global_context.my_keys().dispute_key() {
            Some(key) => key,
            None => self
                .state
                .ctx
                .my_dispute_key_req
                .as_ref()
                .context("My Dispute Key request missing in context")?
                .3
                .as_ref()
                .context("My Signed Dispute Key missing in context")?
                .clone(),
        };

        Ok(signed_pubkey)
    }

    #[allow(unused)]
    fn ctx_my_comm_key(&self) -> Result<SignedPublicKey> {
        let signed_pubkey = match self.global_context.my_keys().comm_key() {
            Some(key) => key,
            None => self
                .state
                .ctx
                .my_comm_key_req
                .as_ref()
                .context("My Communication Key request missing in context")?
                .3
                .as_ref()
                .context("My Signed Communication Key missing in context")?
                .clone(),
        };

        Ok(signed_pubkey)
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

        trace!("Got {key_type} key for member {member_addr}");

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
        let tx_id = TxIdParser::fb_32_to_txid(utxo.txid);

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
        trace!("Sending message to BitVMX: {msg:?}");

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

        trace!("Generated UTXO: tx={}, output={}", fund_res.0.compute_txid(), fund_res.1);

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

            trace!("Registered member {member_addr}");

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

        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: rethink how we store the committee member data in the context, we can unify it in a MemberOfCommittee struct and reduce the number of ctx_xxx methods
        for (idx, cm) in committee.members.iter().enumerate() {
            trace!("Processing member {idx}");

            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: move it to a From trait impl
            let role = if cm.role == 1 {
                ParticipantRole::Prover
            } else if cm.role == 2 {
                ParticipantRole::Verifier
            } else {
                bail!("Invalid member role: {}", cm.role);
            };

            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: mini optimization: do not request my data, it is in context already

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
                committee_idx: idx,
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
    fn start_step(&mut self, next_step: Steps) -> Result<()> {
        debug!("Starting step {:?}", next_step);

        self.state.step = next_step;

        // Execute the entry action for the new state.
        match next_step {
            Steps::Init => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::GetMyCommInfo => {
                self.request_bitvmx_comm_info();
            }
            Steps::GetMyTakeKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_take_pub_key()?,
                true => panic!("Running GetMyTakeKey when MyKeys are already set"),
            },
            Steps::SignMyTakeKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_take_pub_key_signing()?,
                true => panic!("Running SignMyTakeKey when MyKeys are already set"),
            },
            Steps::GetMyDisputeKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_dispute_pub_key()?,
                true => panic!("Running GetMyDisputeKey when MyKeys are already set"),
            },
            Steps::SignMyDisputeKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_dispute_pub_key_signing()?,
                true => panic!("Running SignMyDisputeKey when MyKeys are already set"),
            },
            Steps::GetMyCommKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_comm_pub_key()?,
                true => panic!("Running GetMyCommKey when MyKeys are already set"),
            },
            Steps::SignMyCommKey => match self.global_context.my_keys().is_set() {
                false => self.request_bitvmx_comm_pub_key_signing()?,
                true => panic!("Running SignMyCommKey when MyKeys are already set"),
            },
            Steps::ApplyToStream => {
                self.apply_to_stream()?;
            }
            Steps::DepositP2PData => {
                self.deposit_communication_data()?;
            }
            Steps::SetupTakeAggregatedKey => {
                self.setup_bitvmx_aggregated_take_pubkey()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                self.setup_bitvmx_aggregated_dispute_pubkey()?;
            }
            Steps::DepositAggregatedKey => {
                self.deposit_aggregated_key()?;
            }
            Steps::SetupDisputeCore => {
                self.setup_dispute_core_protocol()?;
            }
            Steps::Done => {
                info!("Committee setup complete for flow {}", self.state.internal_id);
            }
        }
        Ok(())
    }

    fn complete_step(&mut self, data: StepData) -> Result<()> {
        let current_step = self.state.step;

        debug!(
            "Completing step {current_step:?} for flow {}",
            self.state.internal_id
        );
        trace!("Step data: {data:?}");

        match current_step {
            Steps::Init => {
                self.state.ctx.user_input = Some(data.into_user_input()?);
                self.start_step(Steps::GetMyCommInfo)?;
            }
            Steps::GetMyCommInfo => {
                self.state.ctx.my_comm_info = Some(data.into_p2p_address()?);
                if self.global_context.my_keys().is_set() {
                    trace!("Keys already set, proceeding to ApplyToStream");
                    self.start_step(Steps::ApplyToStream)?;
                } else {
                    self.start_step(Steps::GetMyTakeKey)?;
                }
            }
            Steps::GetMyTakeKey => {
                Self::close_pub_key_req(&mut self.state.ctx.my_take_key_req, data)?;
                self.start_step(Steps::SignMyTakeKey)?;
            }
            Steps::SignMyTakeKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_take_key_req, data)?;
                self.start_step(Steps::GetMyDisputeKey)?;
            }
            Steps::GetMyDisputeKey => {
                Self::close_pub_key_req(&mut self.state.ctx.my_dispute_key_req, data)?;
                self.start_step(Steps::SignMyDisputeKey)?;
            }
            Steps::SignMyDisputeKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_dispute_key_req, data)?;
                self.start_step(Steps::GetMyCommKey)?;
            }
            Steps::GetMyCommKey => {
                Self::close_pub_key_req(&mut self.state.ctx.my_comm_key_req, data)?;
                self.start_step(Steps::SignMyCommKey)?;
            }
            Steps::SignMyCommKey => {
                Self::close_pub_key_signing_req(&mut self.state.ctx.my_comm_key_req, data)?;
                self.start_step(Steps::ApplyToStream)?;
            }
            Steps::ApplyToStream => {
                let pending_committee = data.into_committee_pending()?;
                let committee_id: CommitteeId = pending_committee.inner.committeeId.into();

                let im_selected =
                    self.im_selected_to_new_committee(&pending_committee, &committee_id)?;
                if im_selected {
                    self.update_my_committees(pending_committee, &committee_id)?;
                    self.start_step(Steps::DepositP2PData)?;
                } else {
                    debug!("Not selected for committee {committee_id}");
                    self.start_step(Steps::Done)?;
                }
            }
            Steps::DepositP2PData => {
                data.into_all_comm_data_ready()?;
                self.close_communication_data_step()?;
                self.start_step(Steps::SetupTakeAggregatedKey)?;
            }
            Steps::SetupTakeAggregatedKey => {
                Self::close_agg_key_req(&mut self.state.ctx.agg_take_key, data)?;
                self.start_step(Steps::SetupDisputeAggregatedKey)?;
            }
            Steps::SetupDisputeAggregatedKey => {
                Self::close_agg_key_req(&mut self.state.ctx.agg_dispute_key, data)?;
                self.start_step(Steps::DepositAggregatedKey)?;
            }
            Steps::DepositAggregatedKey => {
                self.state.ctx.committee_ready = Some(data.into_committee_ready()?);
                self.start_step(Steps::SetupDisputeCore)?;
            }
            Steps::SetupDisputeCore => {
                let setup_core_state = &mut self.state.ctx.setup_core;
                let missing_responses = Self::close_setup_core_req(setup_core_state, data)?;
                if missing_responses {
                    trace!("Waiting for dispute core setup");
                    self.state.step = Steps::SetupDisputeCore;
                } else {
                    self.start_step(Steps::Done)?;
                }
            }
            Steps::Done => {
                unreachable!("Done step should not be reached in complete_step");
            }
        }
        Ok(())
    }

    fn request_bitvmx_comm_info(&self) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
    }

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_take_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_take_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_take_key_req, &self.bitvmx_broker)
    }

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_dispute_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_dispute_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(
            &mut self.state.ctx.my_dispute_key_req,
            &self.bitvmx_broker,
        )
    }

    fn request_bitvmx_comm_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_comm_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id)
    }

    fn request_bitvmx_comm_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_comm_key_req, &self.bitvmx_broker)
    }

    fn apply_to_stream(&self) -> Result<()> {
        let user_input = self.ctx_user_input()?;

        let utxo_val = user_input.funding_utxo.value;
        let (tx_id, output) = self
            .generate_my_utxo_regtest(utxo_val)
            .context("Generating funding UTXO")?;

        let utxo = UTXO {
            txid: TxIdParser::txid_to_fb_32(tx_id),
            outputIndex: output,
            amount: utxo_val,
        };

        let stream_id = self.state.ctx.get_stream_id()?;

        let my_take_key = self.ctx_my_take_key()?;
        let my_dispute_key = self.ctx_my_dispute_key()?;

        let input = ApplyToStreamInput {
            stream_id: stream_id.clone(),
            role: u8::from(user_input.role),
            take_key: signed_to_committee_public_key(my_take_key.clone())?,
            dispute_key: signed_to_committee_public_key(my_dispute_key.clone())?,
            peer_id: self.ctx_my_comm_info()?.peer_id,
            funding_utxo: utxo,
        };

        debug!("Applying to stream {:?}", input.stream_id);

        match self.rt_sync.run(self.contracts.apply_to_stream(input)) {
            Ok(_) => {
                info!("Applied to stream {stream_id:?} successfully");

                // once a member is selected, public keys should be the same, so we set them in the
                // global context (reset for convenience as it should be idempotent)
                self.global_context.my_keys().set_take_key(my_take_key);
                self.global_context
                    .my_keys()
                    .set_dispute_key(my_dispute_key);
                self.global_context
                    .my_keys()
                    .set_comm_key(self.ctx_my_comm_key()?);

                Ok(())
            }
            Err(e) => {
                bail!("Failed to apply to stream {stream_id:?}: {e}");
            }
        }
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

        debug!(
            "Depositing communication data for committee {}",
            *committee_id
        );
        trace!("Communication data: {communication_data:?}");

        self.rt_sync.run(
            self.contracts
                .deposit_communication_data(DepositCommunicationDataInput {
                    committee_id,
                    communication_data,
                }),
        )
    }

    fn update_my_committees(
        &mut self,
        pending_committee: NewCommitteePendingEvent,
        committee_id: &CommitteeId,
    ) -> Result<()> {
        info!("Selected for committee {committee_id}");
        self.state.ctx.committee_pending = Some(pending_committee);
        let role = self.ctx_user_input()?.role;
        self.global_context
            .my_committees()
            .add(committee_id.clone(), role);
        Ok(())
    }

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()> {
        debug!("Setting up aggregated take key");

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
        debug!("Setting up aggregated dispute key");

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

    fn deposit_aggregated_key(&self) -> Result<()> {
        let aggregated_take_key = self
            .ctx_aggregated_take_key()
            .context("Deposit Aggregated Key")?;

        let committee_id = self.state.ctx.get_committee_id()?;

        let aggregated_key = Bytes::from(aggregated_take_key.to_bytes().to_vec());

        debug!("Depositing aggregated key {} for committee {}",
            hex::encode(&aggregated_key), *committee_id);

        let input = DepositAggregatedKeyInput {
            committee_id,
            aggregated_key,
        };

        self.rt_sync
            .run(self.contracts.deposit_aggregated_key(input))?;

        Ok(())
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
        debug!("Setting up dispute core protocol");

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

        let committee_id = self.state.ctx.get_committee_id()?;

        let protocol_ids = dispute_core.setup(
            committee_id.clone(),
            members,
            p2p_addrs,
            self.ctx_aggregated_take_key()?,
            self.ctx_aggregated_dispute_key()?,
            my_speedup_utxo,
        )?;

        for pid in protocol_ids {
            self.state
                .ctx
                .setup_core
                .push((pid, committee_id.clone(), false))
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
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
}

impl<CG, BC, FactoryBSF> SetupCommitteeProcessor<CG, BC, FactoryBSF>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC>,
{
    pub(crate) fn new(flow_factory: FactoryBSF, global_context: GlobalContext) -> Self {
        Self {
            flow_factory,
            flows: HashMap::new(),
            global_context,
            events_confirming: HashMap::new(),
            blockchain_view: BlockchainView::new(),
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
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: optimize this search by keeping convenient map of stream_id -> internal_id or alike

        self.flows.values_mut().find(|f| {
            f.state
                .ctx
                .get_stream_id()
                .map_or(false, |id| id == stream_id)
        })
    }

    fn get_flow_for_committee_pending(
        &mut self,
        committee_id: CommitteeId,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: optimize this search by keeping convenient map of committee_id -> internal_id or alike

        if !self.global_context.my_committees().im_member(&committee_id) {
            trace!("Skipping committee {committee_id} - not mine");
            return None;
        }

        let pending_committee_flows: Vec<_> = self
            .flows
            .values_mut()
            .filter(|f| {
                f.state
                    .ctx
                    .committee_pending
                    .as_ref()
                    .map_or(false, |ev| ev.inner.committeeId == *committee_id)
            })
            .collect();

        if pending_committee_flows.len() > 1 {
            error!("Multiple flows in status committee_pending for committee {committee_id}");
            None
        } else {
            pending_committee_flows.into_iter().next()
        }
    }

    fn get_flow_for_bitvmx_response(
        &mut self,
        req_id: &Uuid,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC>> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: super naive approach implemented here for now, find within the different flows and their step datas one with the received req_id
        // an alternative could be storing all the requests (ids) for which the flow is waiting response
        // in a same array - but I find this super risky, as it will only work if a) we NEVER send 2
        // "concurrent request-id-depending" messages to BitVMX and b) BitVMX guarantees order in request/response;
        // in addition to that, any change in the code could break it and end up mixing requests/responses/steps

        self.flows.values_mut().find(|flow| {
            Self::pubkey_request_matches(&flow.state.ctx.my_take_key_req, req_id)
                || Self::pubkey_request_matches(&flow.state.ctx.my_dispute_key_req, req_id)
                || Self::pubkey_request_matches(&flow.state.ctx.my_comm_key_req, req_id)
                || Self::aggregated_key_request_matches(&flow.state.ctx.agg_take_key, req_id)
                || Self::aggregated_key_request_matches(&flow.state.ctx.agg_dispute_key, req_id)
                || Self::setup_core_request_matches(
                    &flow.state.ctx.setup_core,
                    req_id,
                    &flow.state.ctx.get_committee_id(),
                )
        })
    }

    /// checks if a pubkey request (either for key generation or signing) matches the given request id
    fn pubkey_request_matches(pubkey_req: &PubKeyReq, req_id: &Uuid) -> bool {
        if let Some((pk_req_id, _, sign_req_id, _)) = pubkey_req {
            pk_req_id == req_id || sign_req_id.map_or(false, |id| id == *req_id)
        } else {
            false
        }
    }

    /// checks if an aggregated key request matches the given request id
    fn aggregated_key_request_matches(agg_key_req: &AggKeyReq, req_id: &Uuid) -> bool {
        if let Some((key_req_id, _)) = agg_key_req {
            key_req_id == req_id
        } else {
            false
        }
    }

    /// checks if any setup core protocol matches the given request id
    fn setup_core_request_matches(
        setup_core: &SetupCoreReq,
        req_id: &Uuid,
        flow_committee_id: &Result<CommitteeId>,
    ) -> bool {
        for (protocol_id, committee_id, _) in setup_core {
            let flow_committee_id = match flow_committee_id {
                Ok(id) => id.clone(),
                Err(_) => {
                    error!("committee_id must exist in setup_core at this step");
                    return false;
                }
            };

            if protocol_id == req_id {
                return if flow_committee_id == *committee_id {
                    true
                } else {
                    error!("Mismatching protocol_id & committee_id in setup_core step");
                    false
                };
            }
        }
        false
    }

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        debug!("Processing confirmed RSK event: {:?}", event);

        let flow_data = match event {
            RskPegManagerEvents::NewCommitteePending(new_committee_pending) => {
                let stream_id = new_committee_pending.inner._committee.streamId;
                let found_flow = self.get_flow_for_stream_id(stream_id.into());
                found_flow.map(|f| (f, StepData::PendingCommittee(new_committee_pending.clone())))
            }
            RskPegManagerEvents::AllCommunicationDataReady(all_comm_data_ready) => {
                let committee_id = all_comm_data_ready.inner._committeeId.into();
                let found_flow = self.get_flow_for_committee_pending(committee_id);
                found_flow.map(|f| {
                    (
                        f,
                        StepData::ReadyCommunicationData(all_comm_data_ready.clone()),
                    )
                })
            }
            RskPegManagerEvents::NewCommitteeReady(new_committee_ready) => {
                let committee_id = new_committee_ready.inner.committeeId.into();
                let found_flow = self.get_flow_for_committee_pending(committee_id);
                found_flow.map(|f| (f, StepData::ReadyCommittee(new_committee_ready.clone())))
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        match flow_data {
            Some((flow, step_data)) => {
                flow.complete_step(step_data)?;
            }
            None => {
                debug!("Received {event:?} but not mine");
            }
        }

        self.close_completed_flows();

        Ok(())
    }

    fn build_new_committee_ready_event_info(
        event: &NewCommitteeReadyEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-ready", event.inner.committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::NewCommitteeReady(event.clone()),
        )
    }
    fn build_all_comm_data_ready_event_info(
        event: &AllCommunicationDataReadyEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-data-ready", event.inner._committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::AllCommunicationDataReady(event.clone()),
        )
    }
    fn build_new_pending_committee_event_info(
        event: &NewCommitteePendingEvent,
    ) -> (String, EventStatus, BlockNumber, RskPegManagerEvents) {
        (
            format!("{}-pending", event.inner.committeeId),
            event.removed,
            event.block_number,
            RskPegManagerEvents::NewCommitteePending(event.clone()),
        )
    }

    fn close_completed_flows(&mut self) {
        let completed: Vec<_> = self
            .flows
            .iter()
            .filter(|(_, flow)| flow.state.step == Steps::Done)
            .map(|(k, _)| *k)
            .collect();

        for key in completed {
            debug!("Removing completed flow: {key:?}");
            self.flows.remove(&key);
        }
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
                let internal_id = Uuid::new_v4();
                let mut flow = self.flow_factory.create_flow(internal_id);
                flow.complete_step(StepData::UserRequest(input.clone()))?;
                self.flows.insert(internal_id, flow);
            }
        }

        Ok(())
    }

    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        // CommInfo is a special case as it does not have a request ID and uses a different flow getter.
        if let OutgoingBitVMXApiMessages::CommInfo(comm_info) = event {
            // we can receive multiple CommInfo events but always for the same member of the
            // committee (the one running the client), but BitVMX will always respond with the
            // same info - so for now we send it to the first flow waiting for it
            if let Some(first_flow) = self.get_first_flow_waiting_comm_info() {
                first_flow.complete_step(StepData::CommInfo(comm_info.clone()))?;
                return Ok(());
            } else {
                trace!("Ignoring CommInfo - not mine")
            };
        }

        // now process all messages with request ID
        let (req_id, step_data) = match event {
            OutgoingBitVMXApiMessages::PubKey(req_id, public_key) => {
                (req_id, StepData::PublicKey(*public_key))
            }
            OutgoingBitVMXApiMessages::SignedMessage(sign_req_id, r, s, rec_id) => {
                (sign_req_id, StepData::SignedMessage(*r, *s, *rec_id))
            }
            OutgoingBitVMXApiMessages::AggregatedPubkey(req_id, pubkey) => {
                (req_id, StepData::PublicKey(*pubkey))
            }
            OutgoingBitVMXApiMessages::SetupCompleted(req_id) => {
                (req_id, StepData::SetupCompleted(req_id.clone()))
            }
            // events that do not trigger a flow step are handled here.
            OutgoingBitVMXApiMessages::Pong() => return Ok(()), // ignored
            _ => {
                trace!("Ignoring BitVMX event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if let Some(flow_for_req_id) = self.get_flow_for_bitvmx_response(req_id) {
            flow_for_req_id.complete_step(step_data)?;
        } else {
            bail!("No flow found for BitVMX event with id {req_id}");
        }

        self.close_completed_flows();

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        // useful for testing purposes
        if REQUIRED_CONFIRMATIONS == 0 {
            return self.process_confirmed_rsk_event(event);
        }

        let (id, is_removal, block_num, managed_event) = match event {
            RskPegManagerEvents::NewCommitteePending(e) => {
                Self::build_new_pending_committee_event_info(e)
            }
            RskPegManagerEvents::AllCommunicationDataReady(e) => {
                Self::build_all_comm_data_ready_event_info(e)
            }
            RskPegManagerEvents::NewCommitteeReady(e) => {
                Self::build_new_committee_ready_event_info(e)
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if is_removal {
            warn!("Removing pending RSK event: {:?}", event);

            // properly clean up the observer before removing the event
            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}")
                }
            } else {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                REQUIRED_CONFIRMATIONS,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event
                .start_confirming(block_num)
                .context("Starting confirming")?;

            self.events_confirming
                .insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting for confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
            return Ok(());
        }

        self.blockchain_view.update(block.clone());

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter_map(|(key, event)| event.is_confirmed().then(|| key.clone()))
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                // properly cleanup the observer before processing the event
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {}: {}", key, e)
                }
                self.process_confirmed_rsk_event(event.get_data())?;
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        // blocks allow periodic cleanup of completed flows, we can improve it with a cleanup task if needed
        self.close_completed_flows();

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
    global_context: GlobalContext,
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
        global_context: GlobalContext,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
        }
    }
}

// TODO commonize with other flows
impl<CG, BC> SetupCommitteeFlowFactoryApi<CG, BC> for SetupCommitteeFlowFactory<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn create_flow(&self, internal_id: Uuid) -> SetupCommitteeFlow<CG, BC> {
        SetupCommitteeFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            self.bitvmx_broker.clone(),
            self.global_context.clone(),
            internal_id,
        )
    }
}

fn signed_to_committee_public_key(spk: SignedPublicKey) -> Result<CommitteeECDSA> {
    let b = spk.public_key.inner.serialize_uncompressed(); // expect 65 bytes: 0x04 || X(32) || Y(32)
    ensure!(b.len() == 65 && b[0] == 0x04, "invalid uncompressed pubkey");
    let (x, y) = b[1..].split_at(32);

    let r = &spk.signature_r;
    let s = &spk.signature_s;
    ensure!(r.len() == 32 && s.len() == 32, "invalid signature length");

    let v = match spk.recovery_id {
        0 | 1 => 27 + spk.recovery_id,
        27 | 28 => spk.recovery_id,
        _ => bail!("invalid recovery_id (expected 0/1 or 27/28)"),
    };

    Ok(CommitteeECDSA {
        x: hex::encode(x),
        y: hex::encode(y),
        r: hex::encode(r),
        s: hex::encode(s),
        v,
    })
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
