use std::any::type_name_of_val;
use std::collections::HashMap;
use std::rc::Rc;

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use anyhow::{Context, Result, bail, ensure};
use bitcoin::key::Parity::Even;
use bitcoin::{Amount, Network, PublicKey, ScriptBuf, Txid, XOnlyPublicKey};
use common::msg_broker::bitvmx_types::{
    Destination, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, OutputType, P2PAddress,
    PartialUtxo, PeerId, SignedPublicKey, Utxo, VariableTypes,
};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use common::types;
use common::types::{BlockNumber, CommitteeId, RskBlockAndUncles, StreamId, TxIdParser};
use log::{debug, error, info, trace, warn};
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};
use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
use transaction_dispatcher::types::{
    ApplyToStreamInput, ApplyToStreamOutput, CommitteeECDSA, DepositAggregatedKeyInput,
    DepositAggregatedKeyOutput, DepositCommunicationDataInput, DepositCommunicationDataOutput,
    GetCommunicationDataInput, GetMemberPublicKeysInput, GetMemberPublicKeysOutput,
    P2PAddressParser,
};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommitteeMember, CommunicationData, UTXO,
};
use uuid::Uuid;

use crate::blockchain_tracker::{BlockchainView, ConfirmableEventWithData};
use crate::config::CommitteeConfig;
use crate::event_processor::EventProcessor;
use crate::flows::committee::dispute_core_setup::DisputeCoreSetup;
use crate::flows::common::{
    COMM_KEY_INDEX, DISPUTE_KEY_INDEX, GlobalContext, TAKE_KEY_INDEX, build_communication_data,
};
use crate::flows::errors::{FailableFlow, FlowError, FlowResultExt};
use crate::store::{
    CoordinatorStoreApi, StoreKey, StorePrefix, cleanup_completed_flows, restore_flows,
};
use crate::types::{
    AllCommunicationDataReadyEvent, EventStatus, MemberOfCommittee, NewCommitteePendingEvent,
    NewCommitteeReadyEvent, RskPegManagerEvents, UserRequests,
};
use crate::user_requests::ApplyToStream;

pub(crate) const NO_LEADER_IDX: u16 = 0;

#[cfg_attr(test, automock)]
trait SetupCommitteeFlowApi {
    fn start_step(&mut self, next_step: Steps) -> Result<(), FlowError>;

    fn complete_step(&mut self, data: StepData) -> Result<(), FlowError>;

    fn request_bitvmx_funding_balance(&mut self);

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
pub(crate) trait SetupCommitteeFlowFactoryApi<
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
>
{
    fn create_flow(&self, internal_id: Uuid) -> SetupCommitteeFlow<CG, BC, S>;
    fn create_flow_from_saved_state(&self, saved_state: State) -> SetupCommitteeFlow<CG, BC, S>;
}

// TODO improve with structs instead of tuples, using tuples for now for validation
type PubKeyReq = Option<(Uuid, Option<PublicKey>, Option<Uuid>, Option<SignedPublicKey>)>; // request id key, raw pub key, req id signing, signed pub key
type AggKeyReq = Option<(Uuid, Option<PublicKey>)>; // request id, response data
type SetupCoreReq = Vec<(Uuid, CommitteeId, bool)>; // request id, committee id, response data
type SendFundsReq = Option<(Uuid, Option<Txid>)>; // request id, funding utxo, speedup utxo

pub(crate) struct FundingUtxos {
    pub speedup: PartialUtxo,
    pub protocol_funding: PartialUtxo,
    pub advance_funds: PartialUtxo,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct FlowContext {
    // stepped
    user_input: Option<ApplyToStream>,
    funding_balance_req: Option<(Uuid, Option<u64>)>, // request id, balance
    my_comm_info: Option<P2PAddress>,
    my_take_key_req: PubKeyReq,
    my_dispute_key_req: PubKeyReq,
    my_comm_key_req: PubKeyReq,
    send_funds_req: SendFundsReq,
    agg_take_key_req: AggKeyReq,
    agg_dispute_key_req: AggKeyReq,
    setup_core_req: SetupCoreReq,
    advance_funds_utxo: Option<PartialUtxo>,
    // async
    committee_pending_ev: Option<NewCommitteePendingEvent>,
    communication_data_ready_ev: Option<Vec<P2PAddress>>,
    committee_ready_req: Option<NewCommitteeReadyEvent>,
}

impl FlowContext {
    fn get_stream_id(&self) -> Result<StreamId> {
        Ok(self.user_input.as_ref().context("Missing stream_id")?.stream_id.clone())
    }

    fn get_committee_id(&self) -> Result<CommitteeId> {
        Ok(self
            .committee_pending_ev
            .as_ref()
            .context("Missing committee pending event")?
            .inner
            .committeeId
            .into())
    }

    fn get_committee_pending_members(&self) -> Result<Vec<CommitteeMember>> {
        let members = self
            .committee_pending_ev
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
            .committee_ready_req
            .as_ref()
            .context("Missing committee ready event")?
            .inner
            ._committee
            .clone();

        Ok(committee)
    }

    fn get_user_input(&self) -> Result<ApplyToStream> {
        self.user_input.as_ref().context("Missing User Input in context").cloned()
    }

    fn get_my_comm_info(&self) -> Result<P2PAddress> {
        let my_comm_info = self.my_comm_info.clone().context("My Comm Info missing in context")?;

        Ok(my_comm_info)
    }

    fn get_aggregated_take_key(&self) -> Result<PublicKey> {
        let agg_take_key = self
            .agg_take_key_req
            .as_ref()
            .context("Aggregated Take Key request missing in context")?
            .1
            .as_ref()
            .context("Aggregated Take Key missing in context")?;

        Ok(*agg_take_key)
    }

    fn get_aggregated_dispute_key(&self) -> Result<PublicKey> {
        let dispute_data_pk = self
            .agg_dispute_key_req
            .as_ref()
            .context("Aggregated Dispute Key request missing in context")?
            .1
            .as_ref()
            .context("Aggregated Dispute Key missing in context")?;

        Ok(*dispute_data_pk)
    }

    fn get_my_communication_data(&self) -> Result<Vec<P2PAddress>> {
        self.communication_data_ready_ev.clone().context("Missing Communication Data in context")
    }

    fn get_my_take_key(&self, global_context: &GlobalContext) -> Result<SignedPublicKey> {
        let signed_pubkey = match global_context.my_keys().take_key() {
            Some(key) => key,
            None => self
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

    fn get_my_dispute_key(&self, global_context: &GlobalContext) -> Result<SignedPublicKey> {
        let signed_pubkey = match global_context.my_keys().dispute_key() {
            Some(key) => key,
            None => self
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

    fn get_my_comm_key(&self, global_context: &GlobalContext) -> Result<SignedPublicKey> {
        let signed_pubkey = match global_context.my_keys().comm_key() {
            Some(key) => key,
            None => self
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

    fn get_my_protocol_utxos(
        &self,
        global_context: &GlobalContext,
        bitcoin_network: Network,
    ) -> Result<FundingUtxos> {
        let txid = self
            .send_funds_req
            .as_ref()
            .context("Missing Send Funds Request")?
            .1
            .context("Missing Send Funds Request TxId")?;

        info!("Funded. Txid: {txid}");
        print_link(txid, bitcoin_network);

        let public_key = self.get_my_dispute_key(global_context)?.public_key;

        let funding_utxo_val = self.get_user_input()?.funding_utxo.value;
        let speedup_utxo_val = self.get_user_input()?.speed_up_utxo.value;
        let advance_funds_utxo_val =
            calculate_advance_funds_value(self.get_user_input()?.advance_funds.value);

        let wpkh = public_key.wpubkey_hash().expect("key is compressed");
        let script_pubkey = ScriptBuf::new_p2wpkh(&wpkh);
        let speedup_ot = OutputType::SegwitPublicKey {
            value: Amount::from_sat(speedup_utxo_val),
            script_pubkey: script_pubkey.clone(),
            public_key,
        };
        let protocol_funding_ot = OutputType::SegwitPublicKey {
            value: Amount::from_sat(funding_utxo_val),
            script_pubkey: script_pubkey.clone(),
            public_key,
        };
        let advance_funds_ot = OutputType::SegwitPublicKey {
            value: Amount::from_sat(advance_funds_utxo_val),
            script_pubkey: script_pubkey.clone(),
            public_key,
        };

        // Output indexes should match the order in the Destination::Batch used in IncomingBitVMXApiMessages::SendFunds
        Ok(FundingUtxos {
            speedup: (txid, 0, Some(speedup_utxo_val), Some(speedup_ot)),
            protocol_funding: (txid, 1, Some(funding_utxo_val), Some(protocol_funding_ot)),
            advance_funds: (txid, 2, Some(advance_funds_utxo_val), Some(advance_funds_ot)),
        })
    }
}

/// Calculates the advance funds UTXO value with a 20% buffer (12/10 = 1.2x).
/// This buffer accounts for potential fee variations and ensures sufficient funds.
fn calculate_advance_funds_value(advance_funds_user_input: u64) -> u64 {
    advance_funds_user_input * 12 / 10
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Steps {
    Init,
    ValidateBalances,
    GetMyCommInfo,
    GetMyTakeKey,
    SignMyTakeKey,
    GetMyDisputeKey,
    SignMyDisputeKey,
    GetMyCommKey,
    SignMyCommKey,
    FundMyBitVmxAccount,
    ApplyToStream,
    DepositP2PData,
    SetupTakeAggregatedKey,
    SetupDisputeAggregatedKey,
    DepositAggregatedKey,
    SetupDisputeCore,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum StepData {
    // sync or member-dependent steps
    UserRequest(ApplyToStream),
    BitVmxFundingBalance(u64),
    CommInfo(P2PAddress),
    PublicKey(PublicKey),
    SignedMessage([u8; 32], [u8; 32], u8), // signature_r, signature_s, recovery_id
    SetupCompleted(Uuid),
    FundsSent(Txid),

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

    fn into_bitvmx_funding_balance(self) -> Result<u64> {
        match self {
            StepData::BitVmxFundingBalance(balance) => Ok(balance),
            _ => bail!("Expected FundingBalance data"),
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

    fn into_funds_sent(self) -> Result<Txid> {
        match self {
            StepData::FundsSent(tx_id) => Ok(tx_id),
            _ => bail!("Expected FundsSent data"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    internal_id: Uuid,
    step: Steps,
    ctx: FlowContext,
}

pub(crate) struct SetupCommitteeFlow<
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    state: State,
    global_context: GlobalContext,
    bitcoin_network: Network,
    store: Rc<S>,
    config: CommitteeConfig,
}

const REGTEST_FEE_RATE: u64 = 10;
const DEFAULT_FEE_RATE: u64 = 1;
pub const ADVANCE_FUNDS_INPUT: &str = "advance_funds_input";

impl<CG, BC, S> SetupCommitteeFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done || self.state.step == Steps::Failed
    }

    fn persist_state(&self) -> Result<()> {
        self.store
            .save_flow(&StoreKey::SetupCommitteeFlow(self.state.internal_id), self.state.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn from_saved_state(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        state: State,
        bitcoin_network: Network,
        store: Rc<S>,
        config: CommitteeConfig,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state,
            global_context,
            bitcoin_network,
            store,
            config,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        contracts: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        internal_id: Uuid,
        bitcoin_network: Network,
        store: Rc<S>,
        config: CommitteeConfig,
    ) -> Self {
        Self {
            contracts,
            rt_sync,
            bitvmx_broker,
            state: State { internal_id, step: Steps::Init, ctx: FlowContext::default() },
            global_context,
            bitcoin_network,
            store,
            config,
        }
    }

    fn my_address(&self) -> types::Address {
        self.contracts.my_address()
    }

    fn request_bitvmx_key_signing(pub_key_req: &mut PubKeyReq, bitvmx_broker: &BC) -> Result<()> {
        let pub_key_req = pub_key_req.as_mut().context("Missing Public Key request")?;

        let pub_key = pub_key_req.1.as_ref().context("Missing Sign Public Key request")?;

        let hash = create_pubkey_hash(pub_key);

        let req_id = Uuid::new_v4();
        pub_key_req.2 = Some(req_id);

        let result = bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SignMessage(req_id, hash.to_vec(), *pub_key),
        );

        if result.is_err() {
            error!("Failed to send msg to BitVMX: {result:?}");
        }

        Ok(())
    }

    fn validate_bitvmx_balance(&mut self, data: StepData) -> Result<()> {
        let balance = data.into_bitvmx_funding_balance()?;

        let r = self
            .state
            .ctx
            .funding_balance_req
            .as_mut()
            .context("Funding balance request missing in context")?;
        r.1 = Some(balance);

        let min_funding_balance = self.config.min_funding_balance;
        if balance < min_funding_balance {
            bail!("Insufficient funding balance: {balance} < {min_funding_balance}")
        }

        debug!("Funding balance check passed: {balance}");

        Ok(())
    }

    fn validate_rsk_balance(&mut self) -> Result<()> {
        let my_address: Address = self.my_address().into();

        debug!("Requesting RSK balance for address: {my_address}");

        let balance_wei = self.rt_sync.run(self.contracts.get_balance())?;

        let min_rsk_balance = self.config.min_rsk_balance;
        // convert wei to a u64 (this is safe for reasonable balance values)
        if balance_wei < U256::from(min_rsk_balance) {
            bail!("Insufficient RSK balance: {balance_wei} < {min_rsk_balance}")
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
        self.state.ctx.communication_data_ready_ev = Some(my_comm_data);
        Ok(())
    }

    fn close_send_funds_req(send_funds_req: &mut SendFundsReq, data: StepData) -> Result<()> {
        let tx_id = data.into_funds_sent()?;

        match send_funds_req {
            Some(r) => {
                r.1 = Some(tx_id);
                Ok(())
            }
            None => {
                bail!("Send Funds request missing in context")
            }
        }
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
        if self.global_context.my_committees().im_member(committee_id) {
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

    fn request_bitvmx_member_pub_key(&self, req_id: Uuid) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetPubKey(req_id, true));
    }

    fn build_funding_utxo(&self) -> Result<UTXO> {
        let funding_utxo = self
            .state
            .ctx
            .get_my_protocol_utxos(&self.global_context, self.bitcoin_network)?
            .protocol_funding;
        let utxo = UTXO {
            txid: TxIdParser::txid_to_fb_32(funding_utxo.0),
            outputIndex: funding_utxo.1,
            amount: funding_utxo.2.context("Missing funding UTXO amount")?,
        };
        Ok(utxo)
    }

    fn get_member_keys_by_type(&self, member_addr: Address, key_index: usize) -> Result<PublicKey> {
        let member = self
            .state
            .ctx
            .get_committee_pending_members()?
            .into_iter()
            .find(|m| m.memberAddress == member_addr)
            .with_context(|| format!("Member {member_addr} not found in committee members"))?;

        self.get_member_key(key_index, &member)
    }

    fn get_member_key(&self, key_index: usize, member: &CommitteeMember) -> Result<PublicKey> {
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

        let key_bytes: FixedBytes<32> =
            key_str.parse().context("Failed to parse public key str to FixedBytes<32>")?;
        let x_only_key = XOnlyPublicKey::from_slice(key_bytes.as_slice())
            .context("Failed to parse aggregated public key")?;

        trace!("Got {key_type} key for member {member_addr}");

        // BitVMX adjusts parity to Even, so we do the same here
        let secp_key = x_only_key.public_key(Even);
        let member_key = PublicKey::new(secp_key);

        Ok(member_key)
    }

    fn build_member_funding_utxo(
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

        Ok((tx_id, utxo.outputIndex, Some(utxo.amount), Some(output_type)))
    }

    fn send_bitvmx_msg(&self, msg: IncomingBitVMXApiMessages) {
        trace!("Sending message to BitVMX: {msg:?}");

        let result = self.bitvmx_broker.send(BROKER_SERVER_ID, msg);
        if result.is_err() {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {result:?}");
        }
    }

    fn get_member_public_keys_from_contracts(
        &self,
        member_address: Address,
    ) -> Result<GetMemberPublicKeysOutput> {
        self.rt_sync
            .run(self.contracts.get_member_public_keys(GetMemberPublicKeysInput { member_address }))
            .map_err(|e| anyhow::anyhow!("Failed to get member public keys: {e}"))
    }

    pub fn fund_protocol(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        let fee_rate = if self.bitcoin_network == Network::Regtest {
            REGTEST_FEE_RATE
        } else {
            DEFAULT_FEE_RATE
        }; // TODO copied from get_fee_rate on BitVMX client

        let public_key = self.state.ctx.get_my_dispute_key(&self.global_context)?.public_key;

        let funding_utxo_val = self.state.ctx.get_user_input()?.funding_utxo.value;
        let speedup_utxo_val = self.state.ctx.get_user_input()?.funding_utxo.value;
        let advance_funds_utxo_val =
            calculate_advance_funds_value(self.state.ctx.get_user_input()?.advance_funds.value);

        info!("Funding dispute pubkey of {} with: {}", req_id, speedup_utxo_val + funding_utxo_val);

        self.state.ctx.send_funds_req = Some((req_id, None));

        let result = self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            IncomingBitVMXApiMessages::SendFunds(
                req_id,
                Destination::Batch(vec![
                    Destination::P2WPKH(public_key, speedup_utxo_val),
                    Destination::P2WPKH(public_key, funding_utxo_val),
                    Destination::P2WPKH(public_key, advance_funds_utxo_val),
                ]),
                Some(fee_rate),
            ),
        );

        if result.is_err() {
            bail!("Failed to send msg to BitVMX: {result:?}");
        }

        Ok(())
    }

    fn build_my_communication_data(&self) -> Result<Vec<P2PAddress>> {
        let committee_id = self.state.ctx.get_committee_id().context("Get Communication Data")?;

        let my_address: Address = self.my_address().into();
        let input = GetCommunicationDataInput {
            // TODO rethink if this is needed or a member should only request its own communication data and therefore this param is not required
            member_address: my_address,
            committee_id,
        };

        let comm_data = self.rt_sync.run(self.contracts.get_committee_communication_data(input))?;

        let committee_addresses = comm_data
            .communication_data
            .into_iter()
            .map(|data| P2PAddressParser::addr_from_contracts(&data))
            .collect::<Result<Vec<_>>>()?;

        let my_p2p_address = self.state.ctx.get_my_comm_info()?.address;

        // temporarily stored PeerId as the communication key, agreed with Fairgate
        let committee_peer_ids = self.get_committee_peer_ids()?;

        build_communication_data(&my_p2p_address, &committee_addresses, &committee_peer_ids)
    }

    fn get_committee_keys_by_type(&self, key_index: usize) -> Result<Vec<PublicKey>> {
        let mut committee_pub_keys = vec![];

        for member in self.state.ctx.get_committee_pending_members()? {
            let member_key = self.get_member_key(key_index, &member)?;
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
            peer_ids.push(PeerId(key_str.clone()));
        }

        Ok(peer_ids)
    }

    fn build_members_of_committee(
        &mut self,
        committee: &Committee,
    ) -> Result<Vec<MemberOfCommittee>> {
        let mut member_of_committee = vec![];

        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: rethink how we store the committee member data in the context, we can unify it in a MemberOfCommittee struct and reduce the number of ctx_xxx methods
        for (idx, cm) in committee.members.iter().enumerate() {
            trace!("Processing member {idx}");

            let role = cm.role.try_into()?;

            let take_key = self.get_member_keys_by_type(cm.memberAddress, TAKE_KEY_INDEX)?;
            let dispute_key = self.get_member_keys_by_type(cm.memberAddress, DISPUTE_KEY_INDEX)?;

            let contracts_utxo =
                committee.fundingUTXOs.get(idx).context("Missing utxo for committee member")?;

            let funding_utxo = Self::build_member_funding_utxo(&dispute_key, contracts_utxo)?;

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

impl<CG, BC, S> FailableFlow for SetupCommitteeFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn fail(&mut self) {
        error!("Marking flow {} as failed and cleaning up", self.state.internal_id);
        self.state.step = Steps::Failed;
    }
}

impl<CG, BC, S> SetupCommitteeFlowApi for SetupCommitteeFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn start_step(&mut self, next_step: Steps) -> Result<(), FlowError> {
        debug!("Starting step {next_step:?}");

        self.state.step = next_step;

        // Execute the entry action for the new state.
        match next_step {
            Steps::Init => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::ValidateBalances => {
                self.validate_rsk_balance().or_transient()?;
                self.request_bitvmx_funding_balance();
            }
            Steps::GetMyCommInfo => {
                self.request_bitvmx_comm_info();
            }
            Steps::GetMyTakeKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyTakeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_take_pub_key()?;
                }
            }
            Steps::SignMyTakeKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyTakeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_take_pub_key_signing()?;
                }
            }
            Steps::GetMyDisputeKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyDisputeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_dispute_pub_key()?;
                }
            }
            Steps::SignMyDisputeKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyDisputeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_dispute_pub_key_signing()?;
                }
            }
            Steps::GetMyCommKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyCommKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_comm_pub_key()?;
                }
            }
            Steps::SignMyCommKey => {
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyCommKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_comm_pub_key_signing()?;
                }
            }
            Steps::FundMyBitVmxAccount => {
                // here we are funding the BitVMX Bitcoin account to complete this protocol
                self.fund_protocol()?;
            }
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
                info!("CommitteeSetupFlow Done: {}", self.state.internal_id);
            }
            Steps::Failed => {
                unreachable!("Failed step should not be reached in start_step");
            }
        }

        // Persist state after successful step completion
        self.persist_state()?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn complete_step(&mut self, data: StepData) -> Result<(), FlowError> {
        let current_step = self.state.step;

        debug!("Completing step {current_step:?} for flow {}", self.state.internal_id);
        trace!("Step data: {data:?}");

        trace!("Flow Context: {:?}", self.state.ctx);
        trace!("Global Context: {:?}", self.global_context);

        // Process the step
        match current_step {
            Steps::Init => {
                self.state.ctx.user_input = Some(data.into_user_input()?);
                self.start_step(Steps::ValidateBalances)?;
            }
            Steps::ValidateBalances => {
                self.validate_bitvmx_balance(data)?;
                self.start_step(Steps::GetMyCommInfo)?;
            }
            Steps::GetMyCommInfo => {
                self.state.ctx.my_comm_info = Some(data.into_p2p_address()?);
                if self.global_context.my_keys().is_set() {
                    debug!("My Keys already set, jumping to FundMyBitVmxAccount step");
                    self.start_step(Steps::FundMyBitVmxAccount)?;
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
                self.start_step(Steps::FundMyBitVmxAccount)?;
            }
            Steps::FundMyBitVmxAccount => {
                Self::close_send_funds_req(&mut self.state.ctx.send_funds_req, data)?;
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
                Self::close_agg_key_req(&mut self.state.ctx.agg_take_key_req, data)?;
                self.start_step(Steps::SetupDisputeAggregatedKey)?;
            }
            Steps::SetupDisputeAggregatedKey => {
                Self::close_agg_key_req(&mut self.state.ctx.agg_dispute_key_req, data)?;
                self.start_step(Steps::DepositAggregatedKey)?;
            }
            Steps::DepositAggregatedKey => {
                self.state.ctx.committee_ready_req = Some(data.into_committee_ready()?);
                self.start_step(Steps::SetupDisputeCore)?;
            }
            Steps::SetupDisputeCore => {
                let setup_core_state = &mut self.state.ctx.setup_core_req;
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
            Steps::Failed => {
                unreachable!("Failed step should not be reached in complete_step");
            }
        }

        Ok(())
    }

    fn request_bitvmx_funding_balance(&mut self) {
        let req_id = Uuid::new_v4();
        self.state.ctx.funding_balance_req = Some((req_id, None));
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetFundingBalance(req_id));
    }

    fn request_bitvmx_comm_info(&self) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo());
    }

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_take_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id);
        Ok(())
    }

    fn request_bitvmx_take_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_take_key_req, &self.bitvmx_broker)
    }

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.state.ctx.my_dispute_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id);
        Ok(())
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
        self.request_bitvmx_member_pub_key(req_id);
        Ok(())
    }

    fn request_bitvmx_comm_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_comm_key_req, &self.bitvmx_broker)
    }

    fn apply_to_stream(&self) -> Result<()> {
        let utxo = self.build_funding_utxo()?;

        let stream_id = self.state.ctx.get_stream_id()?;

        let my_take_key = self.state.ctx.get_my_take_key(&self.global_context)?;
        let my_dispute_key = self.state.ctx.get_my_dispute_key(&self.global_context)?;

        let user_input = self.state.ctx.get_user_input()?;

        let input = ApplyToStreamInput {
            stream_id: stream_id.clone(),
            role: u8::from(user_input.role),
            take_key: signed_to_committee_public_key(&my_take_key)?,
            dispute_key: signed_to_committee_public_key(&my_dispute_key)?,
            peer_id: self.state.ctx.get_my_comm_info()?.peer_id,
            funding_utxo: utxo,
        };

        debug!("Applying to stream {:?}", input.stream_id);

        self.rt_sync.run(self.contracts.apply_to_stream(input)).or_else(|e| match e {
            DomainErrors::MemberAlreadyRegisteredForStream(_) => {
                info!("Member already registered for stream {stream_id:?} - treating as success");
                Ok(ApplyToStreamOutput { transaction_hash: "already_registered".to_string() })
            }
            DomainErrors::NoRevertError(e) => {
                // insuf. funds candidate
                Err(FlowError::transient(e))
            }
            _ => Err(anyhow::Error::from(e))?,
        })?;

        info!("Applied to stream {stream_id:?} successfully");

        // once a member is selected, public keys should be the same, so we set them in the
        // global context (reset for convenience as it should be idempotent)
        self.global_context.my_keys().set_take_key(my_take_key);
        self.global_context.my_keys().set_dispute_key(my_dispute_key);
        self.global_context
            .my_keys()
            .set_comm_key(self.state.ctx.get_my_comm_key(&self.global_context)?);

        Ok(())
    }

    fn deposit_communication_data(&self) -> Result<DepositCommunicationDataOutput> {
        let committee_id =
            self.state.ctx.get_committee_id().context("Deposit Communication Data")?;

        let my_p2p_address = self.state.ctx.get_my_comm_info()?;

        let mut communication_data = vec![];
        // communication data size
        for member in self.state.ctx.get_committee_pending_members()? {
            let my_address: Address = self.my_address().into();
            if member.memberAddress == my_address {
                // contracts require zeroed communication data for my own address on deposit
                communication_data.push(CommunicationData::default());
            } else {
                let data = P2PAddressParser::addr_to_contracts(&my_p2p_address.address)?;
                communication_data.push(data);
            }
        }

        debug!("Depositing communication data for committee {}", *committee_id);
        trace!("Communication data: {communication_data:?}");

        let result = self.rt_sync.run(self.contracts.deposit_communication_data(
            DepositCommunicationDataInput {
                committee_id: committee_id.clone(),
                communication_data,
            },
        )).or_else(|e| {
            match e {
                DomainErrors::MemberAlreadyDepositedCommunicationData(_) => {
                    info!(
                        "Member already deposited communication data for committee {} - treating as success",
                        *committee_id
                    );
                    Ok(DepositCommunicationDataOutput {
                        transaction_hash: "already_deposited".to_string(),
                    })
                }
                DomainErrors::NoRevertError(e) => {
                    // insuf. funds candidate
                    Err(FlowError::transient(e))
                }
                _ => Err(anyhow::Error::from(e))?,
            }
        })?;

        Ok(result)
    }

    fn update_my_committees(
        &mut self,
        pending_committee: NewCommitteePendingEvent,
        committee_id: &CommitteeId,
    ) -> Result<()> {
        info!("Selected for committee {committee_id}");
        self.state.ctx.committee_pending_ev = Some(pending_committee);
        let role = self.state.ctx.get_user_input()?.role;
        self.global_context.my_committees().add(committee_id.clone(), role);
        Ok(())
    }

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()> {
        debug!("Setting up aggregated take key");

        let take_key_id = self.get_take_aggregated_key_id()?;
        self.state.ctx.agg_take_key_req = Some((take_key_id, None));

        let committee_take_keys = self.get_committee_keys_by_type(TAKE_KEY_INDEX)?;
        let communication_data = self.state.ctx.get_my_communication_data()?;

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
        self.state.ctx.agg_dispute_key_req = Some((dispute_key_id, None));

        let committee_dispute_keys = self.get_committee_keys_by_type(DISPUTE_KEY_INDEX)?;
        let communication_data = self.state.ctx.get_my_communication_data()?;

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
        let aggregated_take_key =
            self.state.ctx.get_aggregated_take_key().context("Deposit Aggregated Key")?;

        let committee_id = self.state.ctx.get_committee_id()?;

        let aggregated_key = Bytes::from(aggregated_take_key.to_bytes().clone());

        debug!(
            "Depositing aggregated key {} for committee {}",
            hex::encode(&aggregated_key),
            *committee_id
        );

        let input =
            DepositAggregatedKeyInput { committee_id: committee_id.clone(), aggregated_key };

        self.rt_sync.run(self.contracts.deposit_aggregated_key(input)).or_else(|e| match e {
            DomainErrors::MemberInfoAlreadyDeposited(_) => {
                info!(
                    "Member info already deposited for committee {} - treating as success",
                    *committee_id
                );
                Ok(DepositAggregatedKeyOutput { transaction_hash: "already_deposited".to_string() })
            }
            DomainErrors::NoRevertError(e) => {
                // insuf. funds candidate
                Err(FlowError::transient(e))
            }
            _ => Err(anyhow::Error::from(e))?,
        })?;

        Ok(())
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
        debug!("Setting up dispute core protocol");

        let committee = self.state.ctx.get_committee_ready()?;
        let members = self.build_members_of_committee(&committee)?;

        let dispute_core = DisputeCoreSetup::new(self.bitvmx_broker.clone());

        let partial_utxo = self
            .state
            .ctx
            .get_my_protocol_utxos(&self.global_context, self.bitcoin_network)?
            .speedup;
        let my_speedup_utxo = Utxo {
            txid: partial_utxo.0,
            vout: partial_utxo.1,
            amount: partial_utxo.2.context("Missing speedup UTXO amount")?,
            pub_key: self.state.ctx.get_my_dispute_key(&self.global_context)?.public_key,
        };

        let p2p_addrs = self.state.ctx.get_my_communication_data()?;

        let committee_id = self.state.ctx.get_committee_id()?;

        let protocol_ids = dispute_core.setup(
            &committee_id,
            &members,
            &p2p_addrs,
            self.state.ctx.get_aggregated_take_key()?,
            self.state.ctx.get_aggregated_dispute_key()?,
            my_speedup_utxo,
        )?;

        for pid in protocol_ids {
            self.state.ctx.setup_core_req.push((pid, committee_id.clone(), false));
        }

        let committee_uuid = Uuid::from_u128(*committee_id);
        let advance_funds_utxo = self
            .state
            .ctx
            .get_my_protocol_utxos(&self.global_context, self.bitcoin_network)?
            .advance_funds;

        self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetVar(
            committee_uuid,
            ADVANCE_FUNDS_INPUT.to_string(),
            VariableTypes::Utxo(advance_funds_utxo),
        ));

        Ok(())
    }
}

pub(crate) struct SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, SetupCommitteeFlow<CG, BC, S>>,
    global_context: GlobalContext,
    blockchain_view: BlockchainView,
    events_confirming: HashMap<String, ConfirmableEventWithData>,
    store: Rc<S>,
    required_confirmations: u32,
}

impl<CG, BC, FactoryBSF, S> SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    pub(crate) fn new(
        flow_factory: FactoryBSF,
        global_context: GlobalContext,
        store: &Rc<S>,
        required_confirmations: u32,
    ) -> Self {
        let mut processor = Self {
            flow_factory,
            flows: HashMap::new(),
            global_context,
            events_confirming: HashMap::new(),
            blockchain_view: BlockchainView::new(),
            store: Rc::clone(store),
            required_confirmations,
        };

        // Restore flows from store
        let flow_factory =
            |saved_state: State| processor.flow_factory.create_flow_from_saved_state(saved_state);

        processor.flows =
            restore_flows(store.as_ref(), StorePrefix::SetupCommitteeFlow, flow_factory)
                .expect("Failed to load flows from store");
        processor
    }
}

impl<CG, BC, FactoryBSF, S> SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    fn continue_flow(flow: &mut SetupCommitteeFlow<CG, BC, S>, data: StepData) {
        match flow.complete_step(data) {
            Ok(()) => {
                trace!(
                    "Step {:?} completed successfully for flow {}",
                    flow.state.step, flow.state.internal_id
                );
            }
            Err(FlowError::Fatal { message, .. }) => {
                error!(
                    "Fatal error in flow {} at step {:?}: {}",
                    flow.state.internal_id, flow.state.step, message
                );
                flow.fail();
            }
            Err(FlowError::Transient { message, .. }) => {
                error!(
                    "Transient error in flow {} at step {:?}: {}",
                    flow.state.internal_id, flow.state.step, message
                );
            }
        }
    }

    fn get_first_flow_waiting_comm_info(&mut self) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        // CommInfo
        self.flows.values_mut().find(|f| f.state.step == Steps::GetMyCommInfo)
    }

    fn get_flow_for_stream_id(
        &mut self,
        stream_id: &StreamId,
        expected_step: Steps,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: optimize this search by keeping convenient map of stream_id -> internal_id or alike

        self.flows.values_mut().find(|f| {
            let is_in_expected_step = f.state.step == expected_step;
            let is_flow_for_stream = Self::is_flow_for_stream(f, stream_id);
            is_in_expected_step && is_flow_for_stream
        })
    }

    fn get_flow_for_committee_pending(
        &mut self,
        committee_id: &CommitteeId,
        expected_step: Steps,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: optimize this search by keeping convenient map of committee_id -> internal_id or alike

        if !self.global_context.my_committees().im_member(committee_id) {
            debug!("Skipping committee {committee_id} - not mine");
            return None;
        }

        let pending_committee_flows: Vec<_> = self
            .flows
            .values_mut()
            .filter(|f| {
                let is_in_expected_step = f.state.step == expected_step;
                let is_flow_for_committee = Self::is_flow_for_committee(f, committee_id);
                is_in_expected_step && is_flow_for_committee
            })
            .collect();

        if pending_committee_flows.len() > 1 {
            error!("Multiple flows in status committee_pending for committee {committee_id}");
            None
        } else {
            pending_committee_flows.into_iter().next()
        }
    }

    fn is_flow_for_committee(
        f: &&mut SetupCommitteeFlow<CG, BC, S>,
        committee_id: &CommitteeId,
    ) -> bool {
        f.state
            .ctx
            .committee_pending_ev
            .as_ref()
            .is_some_and(|ev| ev.inner.committeeId == **committee_id)
    }

    fn get_flow_for_bitvmx_response(
        &mut self,
        req_id: &Uuid,
    ) -> Option<&mut SetupCommitteeFlow<CG, BC, S>> {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-256: super naive approach implemented here for now, find within the different flows and their step datas one with the received req_id
        // an alternative could be storing all the requests (ids) for which the flow is waiting response
        // in a same array - but I find this super risky, as it will only work if a) we NEVER send 2
        // "concurrent request-id-depending" messages to BitVMX and b) BitVMX guarantees order in request/response;
        // in addition to that, any change in the code could break it and end up mixing requests/responses/steps

        self.flows.values_mut().find(|flow| {
            Self::funding_balance_request_matches(
                flow.state.ctx.funding_balance_req.as_ref(),
                req_id,
            ) || Self::pubkey_request_matches(&flow.state.ctx.my_take_key_req, req_id)
                || Self::pubkey_request_matches(&flow.state.ctx.my_dispute_key_req, req_id)
                || Self::pubkey_request_matches(&flow.state.ctx.my_comm_key_req, req_id)
                || Self::aggregated_key_request_matches(&flow.state.ctx.agg_take_key_req, req_id)
                || Self::aggregated_key_request_matches(&flow.state.ctx.agg_dispute_key_req, req_id)
                || Self::fund_bitvmx_request_matches(&flow.state.ctx.send_funds_req, req_id)
                || Self::setup_core_request_matches(
                    &flow.state.ctx.setup_core_req,
                    req_id,
                    &flow.state.ctx.get_committee_id(),
                )
        })
    }

    /// checks if a pubkey request (either for key generation or signing) matches the given request id
    fn pubkey_request_matches(pubkey_req: &PubKeyReq, req_id: &Uuid) -> bool {
        if let Some((pk_req_id, _, sign_req_id, _)) = pubkey_req {
            pk_req_id == req_id || sign_req_id.is_some_and(|id| id == *req_id)
        } else {
            false
        }
    }

    /// checks if an aggregated key request matches the given request id
    fn aggregated_key_request_matches(agg_key_req: &AggKeyReq, req_id: &Uuid) -> bool {
        if let Some((key_req_id, _)) = agg_key_req { key_req_id == req_id } else { false }
    }

    fn fund_bitvmx_request_matches(send_funds_req: &SendFundsReq, req_id: &Uuid) -> bool {
        if let Some((fund_req_id, _)) = send_funds_req { fund_req_id == req_id } else { false }
    }

    fn funding_balance_request_matches(
        funding_balance_req: Option<&(Uuid, Option<u64>)>,
        req_id: &Uuid,
    ) -> bool {
        if let Some((balance_req_id, _)) = funding_balance_req {
            balance_req_id == req_id
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
            let flow_committee_id = if let Ok(id) = flow_committee_id {
                id.clone()
            } else {
                error!("committee_id must exist in setup_core at this step");
                return false;
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

    fn process_confirmed_rsk_event(&mut self, event: &RskPegManagerEvents) {
        info!("Processing confirmed RSK event: {event:?}");
        let flow_data = match event {
            RskPegManagerEvents::NewCommitteePending(ncp) => {
                let stream_id: StreamId = ncp.inner._committee.streamId.into();
                let found_flow = self.get_flow_for_stream_id(&stream_id, Steps::ApplyToStream);
                found_flow.map(|f| (f, StepData::PendingCommittee(ncp.clone())))
            }
            RskPegManagerEvents::AllCommunicationDataReady(acdr) => {
                let committee_id: CommitteeId = acdr.inner._committeeId.into();
                let found_flow =
                    self.get_flow_for_committee_pending(&committee_id, Steps::DepositP2PData);
                found_flow.map(|f| (f, StepData::ReadyCommunicationData(acdr.clone())))
            }
            RskPegManagerEvents::NewCommitteeReady(ncr) => {
                let committee_id: CommitteeId = ncr.inner.committeeId.into();
                let found_flow =
                    self.get_flow_for_committee_pending(&committee_id, Steps::DepositAggregatedKey);
                found_flow.map(|f| (f, StepData::ReadyCommittee(ncr.clone())))
            }
            _ => {
                trace!("Ignoring RSK event: {}", type_name_of_val(event));
                return;
            }
        };

        match flow_data {
            Some((flow, step_data)) => {
                Self::continue_flow(flow, step_data);
            }
            None => {
                warn!("Received {event:?} but no matching flow found");
            }
        }
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

    fn is_flow_for_stream(f: &&mut SetupCommitteeFlow<CG, BC, S>, stream_id: &StreamId) -> bool {
        f.state.ctx.get_stream_id().is_ok_and(|id| id == *stream_id)
    }
}

impl<CG, BC, FactoryBSF, S> EventProcessor for SetupCommitteeProcessor<CG, BC, FactoryBSF, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    FactoryBSF: SetupCommitteeFlowFactoryApi<CG, BC, S>,
    S: CoordinatorStoreApi + 'static,
{
    fn process_user_request(&mut self, req: &UserRequests) -> Result<()> {
        info!("Processing user request: {req:?}");
        match req {
            UserRequests::ApplyToStream(input) => {
                let internal_id = Uuid::new_v4();
                let mut flow = self.flow_factory.create_flow(internal_id);

                Self::continue_flow(&mut flow, StepData::UserRequest(input.clone()));

                self.flows.insert(internal_id, flow);
            }
            UserRequests::GetBitVMXFundingAddress => {
                trace!("Ignoring user request: {req:?}");
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
                Self::continue_flow(first_flow, StepData::CommInfo(comm_info.clone()));
            } else {
                trace!("Ignoring CommInfo - not mine");
            }
        }

        // now process all messages with request ID
        let (req_id, step_data) = match event {
            OutgoingBitVMXApiMessages::FundingBalance(req_id, balance) => {
                (req_id, StepData::BitVmxFundingBalance(*balance))
            }
            OutgoingBitVMXApiMessages::PubKey(req_id, public_key) => {
                (req_id, StepData::PublicKey(*public_key))
            }
            OutgoingBitVMXApiMessages::SignedMessage(sign_req_id, r, s, rec_id) => {
                (sign_req_id, StepData::SignedMessage(*r, *s, *rec_id))
            }
            OutgoingBitVMXApiMessages::AggregatedPubkey(req_id, pubkey) => {
                (req_id, StepData::PublicKey(*pubkey))
            }
            OutgoingBitVMXApiMessages::AggregatedPubkeyNotReady(req_id) => {
                bail!("BitVMX cannot aggregate dispute keys for request {req_id}")
            }
            OutgoingBitVMXApiMessages::SetupCompleted(req_id) => {
                (req_id, StepData::SetupCompleted(*req_id))
            }
            OutgoingBitVMXApiMessages::FundsSent(req_id, tx_id) => {
                (req_id, StepData::FundsSent(*tx_id))
            }
            OutgoingBitVMXApiMessages::WalletError(req_id, tx_id) => {
                bail!("BitVMX WalletError for request {req_id}, tx {tx_id}")
            }
            OutgoingBitVMXApiMessages::WalletNotReady(req_id) => {
                bail!("BitVMX WalletNotReady for request {req_id}")
            }
            // events that do not trigger a flow step are handled here.
            OutgoingBitVMXApiMessages::Pong() => return Ok(()), // ignored
            _ => {
                trace!("Ignoring BitVMX event: {}", type_name_of_val(event));
                return Ok(());
            }
        };

        if let Some(flow) = self.get_flow_for_bitvmx_response(req_id) {
            Self::continue_flow(flow, step_data);
        } else {
            debug!("No flow found for BitVMX event with id {req_id}");
        }

        Ok(())
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        // useful for testing purposes
        if self.required_confirmations == 0 {
            self.process_confirmed_rsk_event(event);
            return Ok(());
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
            warn!("Removing pending RSK event: {event:?}");

            // properly clean up the observer before removing the event
            if let Some(mut removed_ev) = self.events_confirming.remove(&id) {
                if let Err(e) = removed_ev.stop_confirming() {
                    error!("Failed to stop confirming for removed event {id}: {e}");
                }
            } else {
                warn!("Tried to remove non-existing pending event with id {id}");
            }
        } else {
            debug!("Adding new pending {event:?}, start confirming at block {block_num}");

            let mut confirmable_event = ConfirmableEventWithData::new(
                id.clone(),
                self.required_confirmations,
                self.blockchain_view.clone(),
                managed_event,
            );

            confirmable_event.start_confirming(block_num).context("Starting confirming")?;

            self.events_confirming.insert(confirmable_event.id(), confirmable_event);

            debug!("Waiting Rootstock confirmations for {id}");
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.events_confirming.is_empty() {
            trace!("No events left to confirm, skipping block");
            return Ok(());
        }

        self.blockchain_view.update(block);

        // process confirmed events while removing them from the hashmap
        // collect the keys of confirmed events first to avoid mutating while iterating
        let confirmed_keys: Vec<_> = self
            .events_confirming
            .iter()
            .filter(|(_, event)| event.is_confirmed())
            .map(|(key, _)| key.clone())
            .collect();

        for key in confirmed_keys {
            if let Some(mut event) = self.events_confirming.remove(&key) {
                debug!("RSK event confirmed, removing pending {key}");
                trace!("Event data: {:?}", event.get_data());
                // properly cleanup the observer before processing the event
                if let Err(e) = event.stop_confirming() {
                    error!("Failed to stop confirming for event {key}: {e}");
                }
                self.process_confirmed_rsk_event(event.get_data());
            }
        }

        if self.events_confirming.is_empty() {
            debug!("No events left to confirm, clearing blockchain view");
            self.blockchain_view.clear();
        }

        // blocks allow periodic cleanup of completed flows, we can improve it with a cleanup task if needed
        cleanup_completed_flows(
            self.store.as_ref(),
            StorePrefix::SetupCommitteeFlow,
            &mut self.flows,
            SetupCommitteeFlow::is_done,
        );

        Ok(())
    }

    fn shutdown(&mut self) {
        // TODO handle shutdown logic if necessary
    }
}

pub(crate) struct SetupCommitteeFlowFactory<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
    bitvmx_broker: Rc<BC>,
    global_context: GlobalContext,
    bitcoin_network: Network,
    store: Rc<S>,
    config: CommitteeConfig,
}

impl<CG, BC, S> SetupCommitteeFlowFactory<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    pub(crate) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        bitvmx_broker: Rc<BC>,
        global_context: GlobalContext,
        bitcoin_network: Network,
        store: Rc<S>,
        config: CommitteeConfig,
    ) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
            bitvmx_broker,
            global_context,
            bitcoin_network,
            store,
            config,
        }
    }
}

// TODO commonize with other flows
impl<CG, BC, S> SetupCommitteeFlowFactoryApi<CG, BC, S> for SetupCommitteeFlowFactory<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn create_flow(&self, internal_id: Uuid) -> SetupCommitteeFlow<CG, BC, S> {
        SetupCommitteeFlow::new(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            self.global_context.clone(),
            internal_id,
            self.bitcoin_network,
            Rc::clone(&self.store),
            self.config.clone(),
        )
    }

    fn create_flow_from_saved_state(&self, saved_state: State) -> SetupCommitteeFlow<CG, BC, S> {
        SetupCommitteeFlow::from_saved_state(
            Rc::clone(&self.contracts_gateway),
            self.rt_sync.clone(),
            Rc::clone(&self.bitvmx_broker),
            self.global_context.clone(),
            saved_state,
            self.bitcoin_network,
            Rc::clone(&self.store),
            self.config.clone(),
        )
    }
}

fn signed_to_committee_public_key(spk: &SignedPublicKey) -> Result<CommitteeECDSA> {
    let pubkey_bytes = spk.public_key.inner.serialize_uncompressed(); // expect 65 bytes: 0x04 || X(32) || Y(32)
    ensure!(pubkey_bytes.len() == 65 && pubkey_bytes[0] == 0x04, "invalid uncompressed pubkey");
    let (x_coord, y_coord) = pubkey_bytes[1..].split_at(32);

    let sig_r = &spk.signature_r;
    let sig_s = &spk.signature_s;
    ensure!(sig_r.len() == 32 && sig_s.len() == 32, "invalid signature length");

    let recovery_id_value = match spk.recovery_id {
        0 | 1 => 27 + spk.recovery_id,
        27 | 28 => spk.recovery_id,
        _ => bail!("invalid recovery_id (expected 0/1 or 27/28)"),
    };

    Ok(CommitteeECDSA {
        x: hex::encode(x_coord),
        y: hex::encode(y_coord),
        r: hex::encode(sig_r),
        s: hex::encode(sig_s),
        v: recovery_id_value,
    })
}

// Helper function to create keccak256 hash of uncompressed public key
fn create_pubkey_hash(public_key: &PublicKey) -> [u8; 32] {
    // Get uncompressed public key coordinates
    let mut pk = *public_key;
    pk.compressed = false;
    let uncompressed_pub_key = pk.to_bytes().split_off(1); // Remove the 0x04 prefix

    // Create keccak256 hash of the uncompressed public key
    let mut keccak = Keccak::v256();
    let mut pub_key_hash = [0u8; 32];
    keccak.update(&uncompressed_pub_key);
    keccak.finalize(&mut pub_key_hash);

    pub_key_hash
}

// Helper function to construct SignedPublicKey from components
fn construct_signed_pubkey(
    public_key: PublicKey,
    signature_r: [u8; 32],
    signature_s: [u8; 32],
    recovery_id: u8,
) -> SignedPublicKey {
    SignedPublicKey { public_key, signature_r, signature_s, recovery_id }
}

fn print_link(txid: Txid, bitcoin_network: Network) {
    if bitcoin_network == Network::Regtest {
        return;
    }

    let url = match bitcoin_network {
        Network::Testnet => format!("https://mempool.space/testnet/tx/{txid}"),
        Network::Bitcoin => format!("https://mempool.space/tx/{txid}"),
        _ => "Unsupported network".to_string(),
    };
    info!("View transaction at: {url}");
}
