use std::collections::HashMap;
use std::rc::Rc;

use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use anyhow::{Context, Result, bail, ensure};
use bitcoin::key::Parity::Even;
use bitcoin::{Amount, Network, PublicKey, ScriptBuf, Txid, XOnlyPublicKey};
use common::msg_broker::bitvmx_types::{
    CommsAddress, Destination, IncomingBitVMXApiMessages, OP_COSIGN_UTXOS, OutputType, PartialUtxo,
    ParticipantRole, PubKeyHash, SignedPublicKey, Utxo, VariableTypes, WT_INIT_CHALLENGE_UTXOS,
    WtInitChallengeUtxos,
};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::types;
use common::types::{CommitteeId, StreamId, TxIdParser};
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
    Committee, CommunicationData, UTXO,
};
use uuid::Uuid;

use crate::config::CommitteeConfig;
use crate::flows::committee::common::{
    CommitteeData, FundingUtxos, get_dispute_pair_aggregated_key_pid,
};
use crate::flows::committee::dispute_channel_setup::{
    DisputeChannelSetup, DisputeChannelSetupRequest,
};
use crate::flows::committee::dispute_core_setup::{AggregatedKeys, DisputeCoreSetup};
use crate::flows::common::{
    COMM_KEY_INDEX, DISPUTE_KEY_INDEX, GlobalContext, TAKE_KEY_INDEX, build_communication_data,
};
use crate::flows::errors::{FailableFlow, FlowError, FlowResultExt};
use crate::store::{CoordinatorStoreApi, StoreKey};
use crate::types::{
    AllCommunicationDataReadyEvent, MemberOfCommittee, NewCommitteePendingEvent,
    NewCommitteeReadyEvent,
};
use crate::user_requests::ApplyToStream;

pub(crate) const NO_LEADER_IDX: u16 = 0;

#[cfg_attr(test, automock)]
pub(crate) trait SetupCommitteeFlowApi {
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

    fn setup_pairwise_keys(&mut self) -> Result<()>;

    fn setup_dispute_core_protocol(&mut self) -> Result<()>;

    fn request_dispute_core_vars(&mut self) -> Result<()>;
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
// aggregation_id, my_index, partner_index, participants (ordered addresses), aggregated_key
type PairwiseKeyReq = Vec<(Uuid, usize, usize, Vec<types::Address>, Option<PublicKey>)>;
type DisputeChannelReq = Vec<DisputeChannelSetupRequest>;
type SetupChannelReq = Vec<(Uuid, bool)>; // dispute channel program id, setup completed

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct FlowContext {
    // stepped
    user_input: Option<ApplyToStream>,
    funding_balance_req: Option<(Uuid, Option<u64>)>, // request id, balance
    my_comm_info: Option<CommsAddress>,
    my_take_key_req: PubKeyReq,
    my_dispute_key_req: PubKeyReq,
    my_comm_key_req: PubKeyReq,
    send_funds_req: SendFundsReq,
    agg_take_key_req: AggKeyReq,
    agg_dispute_key_req: AggKeyReq,
    pairwise_keys_req: PairwiseKeyReq,
    /// Partner address as JSON string -> aggregated key (JSON keys for serde compatibility).
    pairwise_keys: HashMap<String, PublicKey>,
    setup_core_req: SetupCoreReq,
    setup_channel_req: DisputeChannelReq,
    setup_channel_setup_req: SetupChannelReq,
    #[serde(default)]
    committee_data: Option<CommitteeData>,
    // async
    committee_pending_ev: Option<NewCommitteePendingEvent>,
    communication_data_ready_ev: Option<Vec<CommsAddress>>,
    committee_ready_req: Option<NewCommitteeReadyEvent>,
}

impl FlowContext {
    fn get_stream_id(&self) -> Result<StreamId> {
        Ok(self.user_input.as_ref().context("Missing stream_id")?.stream_id.clone())
    }

    fn get_committee_data(&self) -> Result<&CommitteeData> {
        self.committee_data
            .as_ref()
            .context("CommitteeData not yet initialized (NewCommitteePending not received)")
    }

    fn get_user_input(&self) -> Result<ApplyToStream> {
        self.user_input.as_ref().context("Missing User Input in context").cloned()
    }

    fn get_my_comm_info(&self) -> Result<CommsAddress> {
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

    fn get_my_communication_data(&self) -> Result<Vec<CommsAddress>> {
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
pub(crate) enum Steps {
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
    SetupPairwiseKeys,
    SetupDisputeCore,
    RequestDisputeChannelVars,
    DisputeChannelSetup,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StepData {
    // sync or member-dependent steps
    UserRequest(ApplyToStream),
    BitVmxFundingBalance(u64),
    CommInfo(CommsAddress),
    PublicKey(PublicKey),
    PairwiseAggregatedKey(Uuid, PublicKey), // req_id, aggregated_key
    SignedMessage([u8; 32], [u8; 32], u8),  // signature_r, signature_s, recovery_id
    SetupCompleted(Uuid),
    FundsSent(Txid),
    DisputeCoreVariable(Uuid, String, String), // dispute_core_pid, variable_name, json_data

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

    fn into_comms_address(self) -> Result<CommsAddress> {
        match self {
            StepData::CommInfo(addr) => Ok(addr),
            _ => bail!("Expected CommsAddress data"),
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

    fn into_pairwise_aggregated_key(self) -> Result<(Uuid, PublicKey)> {
        match self {
            StepData::PairwiseAggregatedKey(req_id, pubkey) => Ok((req_id, pubkey)),
            _ => bail!("Expected PairwiseAggregatedKey data"),
        }
    }

    fn into_dispute_core_variable(self) -> Result<(Uuid, String, String)> {
        match self {
            StepData::DisputeCoreVariable(pid, name, data) => Ok((pid, name, data)),
            _ => bail!("Expected DisputeCoreVariable data"),
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

impl<CG, BC, S> SetupCommitteeFlow<CG, BC, S>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
    S: CoordinatorStoreApi,
{
    fn ctx(&self) -> &FlowContext {
        &self.state.ctx
    }

    fn ctx_mut(&mut self) -> &mut FlowContext {
        &mut self.state.ctx
    }

    pub fn is_done(&self) -> bool {
        self.state.step == Steps::Done || self.state.step == Steps::Failed
    }

    fn persist_state(&self) -> Result<()> {
        Self::validate_state_serialization(&self.state)
            .context("Flow state serialization failed")?;
        self.store
            .save_flow(&StoreKey::SetupCommitteeFlow(self.state.internal_id), self.state.clone())
            .context("Failed to persist state")?;
        debug!("State persisted for flow {}", self.state.internal_id);
        Ok(())
    }

    fn validate_state_serialization(state: &State) -> Result<()> {
        let mut buf = Vec::new();
        let serializer = &mut serde_json::Serializer::new(&mut buf);
        serde_path_to_error::serialize(state, serializer).map_err(|e| {
            anyhow::anyhow!("Failed to serialize state at {}: {}", e.path(), e.inner())
        })
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

    // --- Accessors for SetupCommitteeProcessor (encapsulation) ---

    pub(crate) fn internal_id(&self) -> Uuid {
        self.state.internal_id
    }

    pub(crate) fn current_step(&self) -> Steps {
        self.state.step
    }

    /// Returns true if this flow is waiting for a `BitVMX` response with the given request id.
    pub(crate) fn is_waiting_for_bitvmx_request(&self, req_id: &Uuid) -> bool {
        Self::funding_balance_request_matches(self.state.ctx.funding_balance_req.as_ref(), req_id)
            || Self::pubkey_request_matches(&self.state.ctx.my_take_key_req, req_id)
            || Self::pubkey_request_matches(&self.state.ctx.my_dispute_key_req, req_id)
            || Self::pubkey_request_matches(&self.state.ctx.my_comm_key_req, req_id)
            || Self::aggregated_key_request_matches(&self.state.ctx.agg_take_key_req, req_id)
            || Self::aggregated_key_request_matches(&self.state.ctx.agg_dispute_key_req, req_id)
            || Self::pairwise_key_request_matches(&self.state.ctx.pairwise_keys_req, req_id)
            || Self::fund_bitvmx_request_matches(&self.state.ctx.send_funds_req, req_id)
            || Self::setup_core_request_matches(
                &self.state.ctx.setup_core_req,
                req_id,
                &self
                    .state
                    .ctx
                    .get_committee_data()
                    .map(|committee_data| committee_data.committee_id.clone()),
            )
            || Self::setup_channel_request_matches(&self.state.ctx.setup_channel_setup_req, req_id)
    }

    pub(crate) fn is_pairwise_aggregated_key_request(&self, req_id: &Uuid) -> bool {
        Self::pairwise_key_request_matches(&self.state.ctx.pairwise_keys_req, req_id)
    }

    pub(crate) fn is_for_stream(&self, stream_id: &StreamId) -> bool {
        self.state.ctx.get_stream_id().is_ok_and(|id| id == *stream_id)
    }

    pub(crate) fn is_for_committee(&self, committee_id: &CommitteeId) -> bool {
        self.state
            .ctx
            .committee_pending_ev
            .as_ref()
            .is_some_and(|ev| ev.inner.committeeId == **committee_id)
    }

    pub(crate) fn is_waiting_for_dispute_core_variable(&self, program_id: &Uuid) -> bool {
        self.state.step == Steps::RequestDisputeChannelVars
            && self
                .state
                .ctx
                .setup_channel_req
                .iter()
                .any(|req| req.dispute_core_pid == *program_id)
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

    fn pubkey_request_matches(pubkey_req: &PubKeyReq, req_id: &Uuid) -> bool {
        if let Some((pk_req_id, _, sign_req_id, _)) = pubkey_req {
            pk_req_id == req_id || sign_req_id.is_some_and(|id| id == *req_id)
        } else {
            false
        }
    }

    fn aggregated_key_request_matches(agg_key_req: &AggKeyReq, req_id: &Uuid) -> bool {
        if let Some((key_req_id, _)) = agg_key_req { key_req_id == req_id } else { false }
    }

    fn fund_bitvmx_request_matches(send_funds_req: &SendFundsReq, req_id: &Uuid) -> bool {
        if let Some((fund_req_id, _)) = send_funds_req { fund_req_id == req_id } else { false }
    }

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

    fn setup_channel_request_matches(setup_channel: &SetupChannelReq, req_id: &Uuid) -> bool {
        setup_channel.iter().any(|(protocol_id, _)| protocol_id == req_id)
    }

    fn pairwise_key_request_matches(pairwise_keys_req: &PairwiseKeyReq, req_id: &Uuid) -> bool {
        pairwise_keys_req.iter().any(|(aggregation_id, _, _, _, _)| aggregation_id == req_id)
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

        let result = bitvmx_broker.send(IncomingBitVMXApiMessages::SignMessage(
            req_id,
            hash.to_vec(),
            *pub_key,
        ));

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
        self.ctx_mut().communication_data_ready_ev = Some(my_comm_data);
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

    fn close_setup_channel_req(
        setup_channel_req: &mut SetupChannelReq,
        data: StepData,
    ) -> Result<bool> {
        let recv_protocol_id = data.into_setup_completed()?;

        for setup_channel in setup_channel_req.iter_mut() {
            if setup_channel.0 == recv_protocol_id {
                setup_channel.1 = true; // mark as completed
            }
        }

        let missing_responses = setup_channel_req.iter().any(|r| !r.1);

        Ok(missing_responses)
    }

    /// Closes a pairwise key request by storing the received aggregated public key.
    /// Returns (`missing_responses`, `my_index`, `partner_index`) for the matched request.
    fn close_pairwise_key_req(
        pairwise_keys_req: &mut PairwiseKeyReq,
        req_id: &Uuid,
        aggregated_key: PublicKey,
    ) -> Result<(bool, usize, usize)> {
        let mut matched_indices: Option<(usize, usize)> = None;

        for pairwise_req in pairwise_keys_req.iter_mut() {
            if pairwise_req.0 == *req_id {
                info!(
                    "Received pairwise aggregated key for request {}: my_index={}, partner_index={}, participants=[{}, {}]",
                    req_id, pairwise_req.1, pairwise_req.2, pairwise_req.3[0], pairwise_req.3[1]
                );
                pairwise_req.4 = Some(aggregated_key); // store the aggregated key
                matched_indices = Some((pairwise_req.1, pairwise_req.2));
            }
        }

        if matched_indices.is_none() {
            let pending_ids = pairwise_keys_req.iter().map(|r| r.0).collect::<Vec<_>>();
            debug!(
                "No matching pairwise key request for req_id {req_id}, pending_ids={pending_ids:?}"
            );
        }

        let (my_index, partner_index) =
            matched_indices.context("No matching pairwise key request found")?;

        // Check if any request is still pending (None)
        let missing_responses = pairwise_keys_req.iter().any(|r| r.4.is_none());
        let received_count = pairwise_keys_req.iter().filter(|r| r.4.is_some()).count();
        let total_count = pairwise_keys_req.len();

        debug!(
            "Pairwise key responses: {received_count}/{total_count} received, missing={missing_responses}"
        );

        Ok((missing_responses, my_index, partner_index))
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

        let committee_id = *self.ctx().get_committee_data()?.committee_id;
        hasher.update(committee_id.to_be_bytes());
        hasher.update("take_aggregated_key");

        // Get the result as a byte array
        let hash = hasher.finalize();
        Uuid::from_slice(&hash[0..16]).context("Failed to convert hash to Uuid")
    }

    fn get_dispute_aggregated_key_id(&self) -> Result<Uuid> {
        let mut hasher = Sha256::new();

        let committee_id = self.ctx().get_committee_data()?.committee_id.clone();
        hasher.update(committee_id.to_be_bytes());
        hasher.update("dispute_aggregated_key");

        // Get the result as a byte array
        let hash = hasher.finalize();
        Uuid::from_slice(&hash[0..16]).context("Failed to convert hash to Uuid")
    }

    fn request_bitvmx_member_pub_key(&self, req_id: Uuid) {
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetEvenPubKey(req_id));
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

    fn parse_member_key(
        key_str: &PubKeyHash,
        key_type: &str,
        member_addr: Address,
    ) -> Result<PublicKey> {
        // TODO revisit this, we are encoding bytes to hex string in the contracts to then decode it back to bytes here

        trace!(
            "Parsing {key_type} key for member {member_addr} (len={}): {key_str}",
            key_str.len()
        );
        let key_bytes: FixedBytes<32> = key_str.parse().with_context(|| {
            format!(
                "Failed to parse {key_type} key to FixedBytes<32> for member {member_addr} (len={})",
                key_str.len()
            )
        })?;
        let x_only_key = XOnlyPublicKey::from_slice(key_bytes.as_slice()).with_context(|| {
            format!("Failed to parse {key_type} x-only key for member {member_addr}")
        })?;

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
        debug!("Sending message to BitVMX: {msg:?}");

        let result = self.bitvmx_broker.send(msg);
        if result.is_err() {
            // TODO(UB-132)
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

        let public_key = self.ctx().get_my_dispute_key(&self.global_context)?.public_key;

        let funding_utxo_val = self.ctx().get_user_input()?.funding_utxo.value;
        let speedup_utxo_val = self.ctx().get_user_input()?.funding_utxo.value;
        let advance_funds_utxo_val =
            calculate_advance_funds_value(self.ctx().get_user_input()?.advance_funds.value);

        info!("Funding dispute pubkey of {} with: {}", req_id, speedup_utxo_val + funding_utxo_val);

        self.ctx_mut().send_funds_req = Some((req_id, None));

        let result = self.bitvmx_broker.send(IncomingBitVMXApiMessages::SendFunds(
            req_id,
            Destination::Batch(vec![
                Destination::P2WPKH(public_key, speedup_utxo_val),
                Destination::P2WPKH(public_key, funding_utxo_val),
                Destination::P2WPKH(public_key, advance_funds_utxo_val),
            ]),
            Some(fee_rate),
        ));

        if result.is_err() {
            bail!("Failed to send msg to BitVMX: {result:?}");
        }

        Ok(())
    }

    fn build_my_communication_data(&self) -> Result<Vec<CommsAddress>> {
        let committee_id =
            self.ctx().get_committee_data().context("Get Communication Data")?.committee_id.clone();

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
            .map(|data| {
                P2PAddressParser::socket_addr_from_contracts(&data)
                    .map(|opt_addr| opt_addr.map(|addr| addr.to_string()).unwrap_or_default())
            })
            .collect::<Result<Vec<_>>>()?;

        let my_p2p_address = self.ctx().get_my_comm_info()?.address.to_string();

        // pubkey_hash stored as the communication key
        let committee_pubkey_hashes = self.get_committee_pubkey_hashes()?;

        build_communication_data(&my_p2p_address, &committee_addresses, &committee_pubkey_hashes)
    }

    fn get_committee_keys_by_type(&self, key_index: usize) -> Result<Vec<PublicKey>> {
        let committee_data = self.ctx().get_committee_data()?;

        committee_data
            .members
            .iter()
            .map(|m| match key_index {
                TAKE_KEY_INDEX => Ok(m.take_key),
                DISPUTE_KEY_INDEX => Ok(m.dispute_key),
                _ => bail!("Invalid key index: {key_index}, expected 0 (take) or 1 (dispute)"),
            })
            .collect()
    }

    fn get_committee_pubkey_hashes(&self) -> Result<Vec<PubKeyHash>> {
        let committee_data = self.ctx().get_committee_data()?;
        let mut pubkey_hashes = vec![];

        for member in &committee_data.members {
            let member_addr: Address = member.address.into();
            let keys = self.get_member_public_keys_from_contracts(member_addr)?;
            let key_str = keys.public_keys.get(COMM_KEY_INDEX).with_context(|| {
                format!("Communication key not found on Committee for {member_addr}")
            })?;

            trace!("Registered member {member_addr}");

            // key_str already decoded
            pubkey_hashes.push(key_str.clone());
        }

        Ok(pubkey_hashes)
    }

    /// Builds and caches `CommitteeData` from the pending committee event.
    /// Must be called after `committee_pending_ev` is set.
    fn build_committee_data(&mut self) -> Result<()> {
        let committee = self
            .state
            .ctx
            .committee_pending_ev
            .as_ref()
            .context("Missing committee pending event")?
            .inner
            ._committee
            .clone();
        let committee_id: CommitteeId = self
            .state
            .ctx
            .committee_pending_ev
            .as_ref()
            .context("Missing committee pending event")?
            .inner
            .committeeId
            .into();
        let members = self.build_members_of_committee(&committee)?;

        self.ctx_mut().committee_data = Some(CommitteeData { committee_id, committee, members });
        Ok(())
    }

    /// Validates that the ready committee event matches the pending committee data.
    /// Ensures committeeId and member list (addresses + order) are consistent.
    fn validate_committee_ready(&self) -> Result<()> {
        let ready_event =
            self.ctx().committee_ready_req.as_ref().context("Missing committee ready event")?;

        let committee_data = self.ctx().get_committee_data()?;

        let ready_committee_id: CommitteeId = ready_event.inner.committeeId.into();
        ensure!(
            committee_data.committee_id == ready_committee_id,
            "Committee ID mismatch: pending={}, ready={}",
            committee_data.committee_id,
            ready_committee_id
        );

        let pending_members = &committee_data.committee.members;
        let ready_members = &ready_event.inner._committee.members;

        ensure!(
            pending_members.len() == ready_members.len(),
            "Member count mismatch: pending={}, ready={}",
            pending_members.len(),
            ready_members.len()
        );

        for (idx, (pending, ready)) in pending_members.iter().zip(ready_members.iter()).enumerate()
        {
            ensure!(
                pending.memberAddress == ready.memberAddress,
                "Member address mismatch at index {}: pending={}, ready={}",
                idx,
                pending.memberAddress,
                ready.memberAddress
            );
            ensure!(
                pending.role == ready.role,
                "Member role mismatch at index {}: pending={}, ready={}",
                idx,
                pending.role,
                ready.role
            );
        }

        info!(
            "Committee ready event validated successfully for committee {}",
            committee_data.committee_id
        );
        Ok(())
    }

    fn build_members_of_committee(&self, committee: &Committee) -> Result<Vec<MemberOfCommittee>> {
        let mut member_of_committee = Vec::with_capacity(committee.members.len());
        for (idx, cm) in committee.members.iter().enumerate() {
            debug!(
                "Processing member idx={}, address={}, role={}, funding_utxos={}",
                idx,
                cm.memberAddress,
                cm.role,
                committee.fundingUTXOs.len()
            );

            let role = cm.role.try_into().with_context(|| {
                format!("Failed to convert member role {} at idx {}", cm.role, idx)
            })?;

            let member_addr = cm.memberAddress;
            let keys = self
                .get_member_public_keys_from_contracts(member_addr)
                .with_context(|| format!("Failed to get keys for member {idx}"))?;
            let take_key_str = keys
                .public_keys
                .get(TAKE_KEY_INDEX)
                .with_context(|| format!("Take key not found on Committee for {member_addr}"))?;
            let dispute_key_str = keys
                .public_keys
                .get(DISPUTE_KEY_INDEX)
                .with_context(|| format!("Dispute key not found on Committee for {member_addr}"))?;
            let take_key = Self::parse_member_key(take_key_str, "Take", member_addr)
                .with_context(|| format!("Failed to parse take key for member {idx}"))?;
            let dispute_key = Self::parse_member_key(dispute_key_str, "Dispute", member_addr)
                .with_context(|| format!("Failed to parse dispute key for member {idx}"))?;

            let contracts_utxo = committee
                .fundingUTXOs
                .get(idx)
                .with_context(|| format!("Missing utxo for committee member {idx}"))?;

            let funding_utxo = Self::build_member_funding_utxo(&dispute_key, contracts_utxo)
                .with_context(|| format!("Failed to build funding utxo for member {idx}"))?;

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

    fn process_pairwise_key_response(&mut self, data: StepData) -> Result<bool> {
        let (req_id, pubkey) = data.into_pairwise_aggregated_key()?;
        debug!(
            "Processing pairwise key response for request {req_id} (pending_requests={})",
            self.ctx().pairwise_keys_req.len()
        );
        let (missing_responses, my_index, partner_index) =
            Self::close_pairwise_key_req(&mut self.ctx_mut().pairwise_keys_req, &req_id, pubkey)?;

        // Get partner address and store the pairwise key
        let comm_data = self.ctx().get_my_communication_data()?;
        debug!(
            "Pairwise key response comm_data_len={}, my_index={}, partner_index={}",
            comm_data.len(),
            my_index,
            partner_index
        );
        let partner_address = comm_data
            .get(partner_index)
            .with_context(|| {
                format!(
                    "Partner address not found at index {partner_index} (comm_data_len={})",
                    comm_data.len()
                )
            })?
            .clone();

        let pairwise_key = pubkey;
        let key = serde_json::to_string(&partner_address)
            .context("Serialize CommsAddress for pairwise_keys key")?;
        self.ctx_mut().pairwise_keys.insert(key, pairwise_key);

        let committee_id_uuid = self.ctx().get_committee_data()?.committee_uuid();
        let var_name = get_dispute_pair_key_name(my_index, partner_index);
        debug!(
            "Sending pairwise key to BitVMX: {committee_id_uuid}, {var_name}, {}",
            hex::encode(pairwise_key.to_bytes())
        );
        self.bitvmx_broker
            .send(IncomingBitVMXApiMessages::SetVar(
                committee_id_uuid,
                var_name,
                VariableTypes::PubKey(pairwise_key),
            ))
            .context("Failed to send pairwise key to BitVMX")?;

        Ok(missing_responses)
    }

    fn process_dispute_core_variable(
        &mut self,
        dispute_core_pid: Uuid,
        var_name: &str,
        var_data: &str,
    ) -> Result<bool> {
        debug!("Processing DisputeCore variable: {var_name} for pid {dispute_core_pid}");

        // Find the request for this dispute_core_pid
        let request = self
            .state
            .ctx
            .setup_channel_req
            .iter_mut()
            .find(|req| req.dispute_core_pid == dispute_core_pid)
            .context("No DisputeChannelSetupRequest found for this dispute_core_pid")?;

        match var_name {
            OP_COSIGN_UTXOS => {
                let utxos: Vec<Option<PartialUtxo>> = serde_json::from_str(var_data)
                    .context("Failed to deserialize OP_COSIGN_UTXOS")?;
                request.op_cosign_utxos = Some(utxos);
                debug!("Received OP_COSIGN_UTXOS for pid {dispute_core_pid}");
            }
            WT_INIT_CHALLENGE_UTXOS => {
                let utxos: Vec<Option<WtInitChallengeUtxos>> = serde_json::from_str(var_data)
                    .context("Failed to deserialize WT_INIT_CHALLENGE_UTXOS")?;
                request.wt_init_challenge_utxos = Some(utxos);
                debug!("Received WT_INIT_CHALLENGE_UTXOS for pid {dispute_core_pid}");
            }
            _ => {
                warn!("Unknown variable name: {var_name}");
            }
        }

        // Check if all requests have all their data
        let missing_responses = self
            .state
            .ctx
            .setup_channel_req
            .iter()
            .any(|req| req.op_cosign_utxos.is_none() || req.wt_init_challenge_utxos.is_none());

        Ok(missing_responses)
    }

    fn complete_dispute_channel_setup(&mut self) -> Result<usize> {
        debug!("Completing DisputeChannel setup");

        let committee_data = self.ctx().get_committee_data()?;

        let p2p_addrs = self.ctx().get_my_communication_data()?;

        // Find my index in the committee
        let my_address: types::Address = self.my_address();
        let my_index = committee_data
            .members
            .iter()
            .position(|m| m.address == my_address)
            .context("My address not found in committee members")?;

        let dispute_channel_setup = DisputeChannelSetup::new(
            self.bitvmx_broker.clone(),
            self.config.drp_program_definition.clone(),
        );

        let protocol_ids = dispute_channel_setup.complete_setup(
            committee_data,
            my_index,
            &p2p_addrs,
            &self.ctx().pairwise_keys,
            &self.ctx().setup_channel_req,
        )?;

        self.ctx_mut().setup_channel_setup_req =
            protocol_ids.iter().map(|id| (*id, false)).collect();

        Ok(protocol_ids.len())
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
    #[allow(clippy::too_many_lines)]
    fn start_step(&mut self, next_step: Steps) -> Result<(), FlowError> {
        debug!("Starting step {next_step:?}");

        self.state.step = next_step;

        // Execute the entry action for the new state.
        match next_step {
            Steps::Init => {
                unreachable!("Init step should not be reached in start_step");
            }
            Steps::ValidateBalances => {
                debug!("CommitteeSetupFlow start validating balances: {}", self.state.internal_id);
                self.validate_rsk_balance().or_transient()?;
                self.request_bitvmx_funding_balance();
            }
            Steps::GetMyCommInfo => {
                debug!("CommitteeSetupFlow start getting MyCommInfo");
                self.request_bitvmx_comm_info();
            }
            Steps::GetMyTakeKey => {
                debug!("CommitteeSetupFlow start getting MyTakeKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyTakeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_take_pub_key()?;
                }
            }
            Steps::SignMyTakeKey => {
                debug!("CommitteeSetupFlow start signing MyTakeKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyTakeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_take_pub_key_signing()?;
                }
            }
            Steps::GetMyDisputeKey => {
                debug!("CommitteeSetupFlow start getting MyDisputeKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyDisputeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_dispute_pub_key()?;
                }
            }
            Steps::SignMyDisputeKey => {
                debug!("CommitteeSetupFlow start signing MyDisputeKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyDisputeKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_dispute_pub_key_signing()?;
                }
            }
            Steps::GetMyCommKey => {
                debug!("CommitteeSetupFlow start getting MyCommKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running GetMyCommKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_comm_pub_key()?;
                }
            }
            Steps::SignMyCommKey => {
                debug!("CommitteeSetupFlow start signing MyCommKey");
                if self.global_context.my_keys().is_set() {
                    panic!("Running SignMyCommKey when MyKeys are already set");
                } else {
                    self.request_bitvmx_comm_pub_key_signing()?;
                }
            }
            Steps::FundMyBitVmxAccount => {
                debug!("CommitteeSetupFlow start funding MyBitVmxAccount");
                // here we are funding the BitVMX Bitcoin account to complete this protocol
                self.fund_protocol()?;
            }
            Steps::ApplyToStream => {
                debug!("CommitteeSetupFlow start apply to stream");
                self.apply_to_stream()?;
            }
            Steps::DepositP2PData => {
                debug!("CommitteeSetupFlow start deposit P2PData");
                self.deposit_communication_data()?;
            }
            Steps::SetupTakeAggregatedKey => {
                debug!("CommitteeSetupFlow start setup taking aggregated key");
                self.setup_bitvmx_aggregated_take_pubkey()?;
            }
            Steps::SetupDisputeAggregatedKey => {
                debug!("CommitteeSetupFlow start setup dispute aggregated key");
                self.setup_bitvmx_aggregated_dispute_pubkey()?;
            }
            Steps::DepositAggregatedKey => {
                debug!("CommitteeSetupFlow start deposit aggregated key");
                self.deposit_aggregated_key()?;
            }
            Steps::SetupPairwiseKeys => {
                debug!("CommitteeSetupFlow start setup pairwise keys");
                self.setup_pairwise_keys()?;
                // Wait for AggregatedPubkey responses from BitVMX
            }
            Steps::SetupDisputeCore => {
                debug!("CommitteeSetupFlow start setup dispute core");
                self.setup_dispute_core_protocol()?;
                debug!("Start Step SetupDisputeCore completed");
            }
            Steps::RequestDisputeChannelVars => {
                debug!("CommitteeSetupFlow start setup dispute channel");
                self.request_dispute_core_vars()?;
            }
            Steps::DisputeChannelSetup => {
                debug!("CommitteeSetupFlow waiting for DisputeChannel setup completion");
                let req_count = self.complete_dispute_channel_setup()?;
                trace!("Requested {req_count} DisputeChannel Setup");
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
        debug!("State persisted after step {next_step:?}");
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn complete_step(&mut self, data: StepData) -> Result<(), FlowError> {
        let current_step = self.state.step;

        debug!("Completing step {current_step:?} for flow {}", self.state.internal_id);
        debug!("Step data: {data:?}");

        debug!("Flow Context: {:?}", self.ctx());
        debug!("Global Context: {:?}", self.global_context);

        // Process the step
        match current_step {
            Steps::Init => {
                debug!("Init");
                self.ctx_mut().user_input = Some(data.into_user_input()?);
                self.start_step(Steps::ValidateBalances)?;
            }
            Steps::ValidateBalances => {
                debug!("CommitteeSetupFlow complete ValidateBalances");
                self.validate_bitvmx_balance(data)?;
                self.start_step(Steps::GetMyCommInfo)?;
            }
            Steps::GetMyCommInfo => {
                debug!("CommitteeSetupFlow complete GetMyCommInfo");
                self.ctx_mut().my_comm_info = Some(data.into_comms_address()?);
                if self.global_context.my_keys().is_set() {
                    debug!("My Keys already set, jumping to FundMyBitVmxAccount step");
                    self.start_step(Steps::FundMyBitVmxAccount)?;
                } else {
                    self.start_step(Steps::GetMyTakeKey)?;
                }
            }
            Steps::GetMyTakeKey => {
                debug!("CommitteeSetupFlow complete GetMyTakeKey");
                Self::close_pub_key_req(&mut self.ctx_mut().my_take_key_req, data)?;
                self.start_step(Steps::SignMyTakeKey)?;
            }
            Steps::SignMyTakeKey => {
                debug!("CommitteeSetupFlow complete SignMyTakeKey");
                Self::close_pub_key_signing_req(&mut self.ctx_mut().my_take_key_req, data)?;
                self.start_step(Steps::GetMyDisputeKey)?;
            }
            Steps::GetMyDisputeKey => {
                debug!("CommitteeSetupFlow complete GetMyDisputeKey");
                Self::close_pub_key_req(&mut self.ctx_mut().my_dispute_key_req, data)?;
                self.start_step(Steps::SignMyDisputeKey)?;
            }
            Steps::SignMyDisputeKey => {
                debug!("CommitteeSetupFlow complete SignMyDisputeKey");
                Self::close_pub_key_signing_req(&mut self.ctx_mut().my_dispute_key_req, data)?;
                self.start_step(Steps::GetMyCommKey)?;
            }
            Steps::GetMyCommKey => {
                debug!("CommitteeSetupFlow complete GetMyCommKey");
                Self::close_pub_key_req(&mut self.ctx_mut().my_comm_key_req, data)?;
                self.start_step(Steps::SignMyCommKey)?;
            }
            Steps::SignMyCommKey => {
                debug!("CommitteeSetupFlow complete SignMyCommKey");
                Self::close_pub_key_signing_req(&mut self.ctx_mut().my_comm_key_req, data)?;
                self.start_step(Steps::FundMyBitVmxAccount)?;
            }
            Steps::FundMyBitVmxAccount => {
                debug!("CommitteeSetupFlow complete FundMyBitVmxAccount");
                Self::close_send_funds_req(&mut self.ctx_mut().send_funds_req, data)?;
                self.start_step(Steps::ApplyToStream)?;
            }
            Steps::ApplyToStream => {
                debug!("CommitteeSetupFlow complete ApplyToStream");
                let pending_committee = data.into_committee_pending()?;
                let committee_id: CommitteeId = pending_committee.inner.committeeId.into();

                let im_selected =
                    self.im_selected_to_new_committee(&pending_committee, &committee_id)?;
                if im_selected {
                    self.update_my_committees(pending_committee, &committee_id)?;
                    self.build_committee_data()?;
                    self.start_step(Steps::DepositP2PData)?;
                } else {
                    debug!("Not selected for committee {committee_id}");
                    self.start_step(Steps::Done)?;
                }
            }
            Steps::DepositP2PData => {
                debug!("CommitteeSetupFlow complete DepositP2PData");
                data.into_all_comm_data_ready()?;
                self.close_communication_data_step()?;
                self.start_step(Steps::SetupTakeAggregatedKey)?;
            }
            Steps::SetupTakeAggregatedKey => {
                debug!("CommitteeSetupFlow complete SetupTakeAggregatedKey");
                Self::close_agg_key_req(&mut self.ctx_mut().agg_take_key_req, data)?;
                self.start_step(Steps::SetupDisputeAggregatedKey)?;
            }
            Steps::SetupDisputeAggregatedKey => {
                debug!("CommitteeSetupFlow complete SetupDisputeAggregatedKey");
                Self::close_agg_key_req(&mut self.ctx_mut().agg_dispute_key_req, data)?;
                self.start_step(Steps::DepositAggregatedKey)?;
            }
            Steps::DepositAggregatedKey => {
                debug!("CommitteeSetupFlow complete DepositAggregatedKey");
                self.ctx_mut().committee_ready_req = Some(data.into_committee_ready()?);
                self.validate_committee_ready()?;
                self.start_step(Steps::SetupPairwiseKeys)?;
            }
            Steps::SetupPairwiseKeys => {
                debug!("CommitteeSetupFlow complete SetupPairwiseKeys");
                let missing_responses = self.process_pairwise_key_response(data)?;

                if missing_responses {
                    debug!("Waiting for more pairwise aggregated key responses");
                    self.state.step = Steps::SetupPairwiseKeys;
                } else {
                    info!("All pairwise aggregated keys received, proceeding to SetupDisputeCore");
                    self.start_step(Steps::SetupDisputeCore)?;
                }
                debug!("Setup PairwiseKeys completed");
            }
            Steps::SetupDisputeCore => {
                debug!("CommitteeSetupFlow complete SetupDisputeCore");
                let setup_core_state = &mut self.ctx_mut().setup_core_req;
                let missing_responses = Self::close_setup_core_req(setup_core_state, data)?;
                if missing_responses {
                    debug!("Waiting for dispute core setup");
                    self.state.step = Steps::SetupDisputeCore;
                } else {
                    self.start_step(Steps::RequestDisputeChannelVars)?;
                }
            }
            Steps::RequestDisputeChannelVars => {
                debug!("CommitteeSetupFlow complete RequestDisputeChannelVars");
                let (dispute_core_pid, var_name, var_data) = data.into_dispute_core_variable()?;

                let missing_responses =
                    self.process_dispute_core_variable(dispute_core_pid, &var_name, &var_data)?;

                if missing_responses {
                    debug!("Waiting for more DisputeCore variables");
                    self.state.step = Steps::RequestDisputeChannelVars;
                } else {
                    info!("All DisputeCore variables received, completing DisputeChannel setup");
                    self.start_step(Steps::DisputeChannelSetup)?;
                }
            }
            Steps::DisputeChannelSetup => {
                debug!("CommitteeSetupFlow complete DisputeChannelSetup");
                let missing_responses = Self::close_setup_channel_req(
                    &mut self.ctx_mut().setup_channel_setup_req,
                    data,
                )?;

                if missing_responses {
                    debug!("Waiting for DisputeChannel setup completion");
                    self.state.step = Steps::DisputeChannelSetup;
                } else {
                    info!("All DisputeChannel setups completed");
                    self.start_step(Steps::Done)?;
                }
            }
            Steps::Done => {
                debug!("CommitteeSetupFlow complete Done");
                unreachable!("Done step should not be reached in complete_step");
            }
            Steps::Failed => {
                debug!("CommitteeSetupFlow complete Failed");
                unreachable!("Failed step should not be reached in complete_step");
            }
        }

        Ok(())
    }

    fn request_bitvmx_funding_balance(&mut self) {
        let req_id = Uuid::new_v4();
        self.ctx_mut().funding_balance_req = Some((req_id, None));
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetFundingBalance(req_id));
    }

    fn request_bitvmx_comm_info(&self) {
        let req_id = Uuid::new_v4();
        self.send_bitvmx_msg(IncomingBitVMXApiMessages::GetCommInfo(req_id));
    }

    fn request_bitvmx_take_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.ctx_mut().my_take_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id);
        Ok(())
    }

    fn request_bitvmx_take_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_take_key_req, &self.bitvmx_broker)
    }

    fn request_bitvmx_dispute_pub_key(&mut self) -> Result<()> {
        let req_id = Uuid::new_v4();
        self.ctx_mut().my_dispute_key_req = Some((req_id, None, None, None));
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
        self.ctx_mut().my_comm_key_req = Some((req_id, None, None, None));
        self.request_bitvmx_member_pub_key(req_id);
        Ok(())
    }

    fn request_bitvmx_comm_pub_key_signing(&mut self) -> Result<()> {
        Self::request_bitvmx_key_signing(&mut self.state.ctx.my_comm_key_req, &self.bitvmx_broker)
    }

    fn apply_to_stream(&self) -> Result<()> {
        let my_address: Address = self.my_address().into();
        let is_whitelisted = self.rt_sync.run(self.contracts.is_whitelisted())?;
        if !is_whitelisted {
            bail!(
                "Member address {my_address} is not whitelisted in the CommitteeRegistry contract"
            );
        }
        info!("Whitelist check passed for address {my_address}");

        let utxo = self.build_funding_utxo()?;

        let stream_id = self.ctx().get_stream_id()?;

        let my_take_key = self.ctx().get_my_take_key(&self.global_context)?;
        let my_dispute_key = self.ctx().get_my_dispute_key(&self.global_context)?;

        let user_input = self.ctx().get_user_input()?;

        let input = ApplyToStreamInput {
            stream_id: stream_id.clone(),
            role: u8::from(user_input.role),
            take_key: signed_to_committee_public_key(&my_take_key)?,
            dispute_key: signed_to_committee_public_key(&my_dispute_key)?,
            pubkey_hash: self.ctx().get_my_comm_info()?.pubkey_hash,
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
            .set_comm_key(self.ctx().get_my_comm_key(&self.global_context)?);

        Ok(())
    }

    fn deposit_communication_data(&self) -> Result<DepositCommunicationDataOutput> {
        let committee_id = self
            .state
            .ctx
            .get_committee_data()
            .context("Deposit Communication Data")?
            .committee_id
            .clone();

        let my_p2p_address = self.ctx().get_my_comm_info()?;

        let committee_data = self.ctx().get_committee_data()?;
        let my_address = self.my_address();

        let mut communication_data = vec![];
        // communication data size
        for member in &committee_data.members {
            if member.address == my_address {
                // contracts require zeroed communication data for my own address on deposit
                communication_data.push(CommunicationData::default());
            } else {
                let data = P2PAddressParser::socket_addr_to_contracts(&my_p2p_address.address)?;
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
        self.ctx_mut().committee_pending_ev = Some(pending_committee);
        self.ctx_mut().committee_data = None;
        let role = self.ctx().get_user_input()?.role;
        self.global_context.my_committees().add(committee_id.clone(), role);
        Ok(())
    }

    fn setup_bitvmx_aggregated_take_pubkey(&mut self) -> Result<()> {
        debug!("Setting up aggregated take key");

        let take_key_id = self.get_take_aggregated_key_id()?;
        self.ctx_mut().agg_take_key_req = Some((take_key_id, None));

        let committee_take_keys = self.get_committee_keys_by_type(TAKE_KEY_INDEX)?;
        let communication_data = self.ctx().get_my_communication_data()?;

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
        self.ctx_mut().agg_dispute_key_req = Some((dispute_key_id, None));

        let committee_dispute_keys = self.get_committee_keys_by_type(DISPUTE_KEY_INDEX)?;
        let communication_data = self.ctx().get_my_communication_data()?;

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
            self.ctx().get_aggregated_take_key().context("Deposit Aggregated Key")?;

        let committee_id = self.ctx().get_committee_data()?.committee_id.clone();

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

    fn setup_pairwise_keys(&mut self) -> Result<()> {
        debug!("Setting up pairwise keys");

        let committee_data = self.ctx().get_committee_data()?;
        let my_address: types::Address = self.my_address();
        let communication_data = self.ctx().get_my_communication_data()?;

        debug!(
            "Setting up pairwise keys for committee {} with {} members",
            *committee_data.committee_id,
            committee_data.members.len()
        );

        // Find my index in the committee
        let my_index = committee_data
            .members
            .iter()
            .position(|m| m.address == my_address)
            .context("My address not found in committee members")?;

        let my_role = committee_data.members[my_index].role.clone();

        debug!(
            "My position in committee: index={my_index}, address={my_address}, role={my_role:?}"
        );

        let mut pairwise_requests: PairwiseKeyReq = Vec::new();

        for (partner_index, partner) in committee_data.members.iter().enumerate() {
            let partner_address = partner.address;
            let partner_role = partner.role.clone();

            debug!(
                "Evaluating partner: index={partner_index}, address={partner_address}, role={partner_role:?}"
            );

            // Skip myself
            if partner_index == my_index {
                debug!("Skipping index {partner_index}: this is myself");
                continue;
            }

            // Skip if neither member is a Prover
            if my_role != ParticipantRole::Prover && partner_role != ParticipantRole::Prover {
                debug!(
                    "Skipping pairwise key: my_index={my_index} ({my_role:?}) <-> partner_index={partner_index} ({partner_role:?}) - neither is Prover"
                );
                continue;
            }

            // Setup participants in deterministic order, the one with the lower index first
            let (idx_first, idx_second) = if my_index < partner_index {
                (my_index, partner_index)
            } else {
                (partner_index, my_index)
            };

            let participants_addresses = if my_index < partner_index {
                vec![my_address, partner_address]
            } else {
                vec![partner_address, my_address]
            };

            // Deterministic id: committee_id + ordered indices + tag
            let aggregation_id = get_dispute_pair_aggregated_key_pid(
                committee_data.committee_uuid(),
                my_index,
                partner_index,
            )?;

            info!(
                "Creating pairwise key request: aggregation_id={}, indices=({}, {}), participants=[{}, {}]",
                aggregation_id,
                idx_first,
                idx_second,
                participants_addresses[0],
                participants_addresses[1]
            );

            // Build participants CommsAddress in deterministic order for BitVMX
            let participants_comms =
                vec![communication_data[idx_first].clone(), communication_data[idx_second].clone()];

            info!(
                "Sending SetupKey to BitVMX: id={aggregation_id}, participants=[{:?}, {:?}]",
                participants_comms[0].address, participants_comms[1].address
            );

            // Send SetupKey message to BitVMX
            self.send_bitvmx_msg(IncomingBitVMXApiMessages::SetupKey(
                aggregation_id,
                participants_comms,
                None,          // participant_keys = None for pairwise
                NO_LEADER_IDX, // leader_idx = 0
            ));

            // Store request with indices, addresses, and None for aggregated_key (to be filled on response)
            pairwise_requests.push((
                aggregation_id,
                my_index,
                partner_index,
                participants_addresses,
                None,
            ));
        }

        info!(
            "Sent {} pairwise key requests for committee {}, waiting for responses",
            pairwise_requests.len(),
            *committee_data.committee_id
        );

        self.ctx_mut().pairwise_keys_req = pairwise_requests;

        Ok(())
    }

    fn setup_dispute_core_protocol(&mut self) -> Result<()> {
        debug!("Setting up dispute core protocol");

        let committee_data = self.ctx().get_committee_data()?;

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
            pub_key: self.ctx().get_my_dispute_key(&self.global_context)?.public_key,
        };

        let p2p_addrs = self.ctx().get_my_communication_data()?;

        let stream_id = self.ctx().get_stream_id()?;

        let advance_funds_utxo = self
            .state
            .ctx
            .get_my_protocol_utxos(&self.global_context, self.bitcoin_network)?
            .advance_funds;

        debug!(
            "DisputeCore setup inputs: committee_id={}, stream_id={}, members={}, speedup_utxo_vout={}, advance_funds_vout={}",
            *committee_data.committee_id,
            *stream_id,
            committee_data.members.len(),
            my_speedup_utxo.vout,
            advance_funds_utxo.1
        );

        let committee_id = committee_data.committee_id.clone();
        let protocol_ids = dispute_core.setup(
            committee_data,
            &p2p_addrs,
            AggregatedKeys {
                take: self.ctx().get_aggregated_take_key()?,
                dispute: self.ctx().get_aggregated_dispute_key()?,
            },
            my_speedup_utxo,
            *stream_id,
            advance_funds_utxo,
        )?;

        for pid in protocol_ids {
            self.ctx_mut().setup_core_req.push((pid, committee_id.clone(), false));
        }
        debug!("DisputeCoreSetup protocol reach end");
        Ok(())
    }

    fn request_dispute_core_vars(&mut self) -> Result<()> {
        debug!("Setting up dispute channel protocol");

        let committee_data = self.ctx().get_committee_data()?;

        // Find my index in the committee
        let my_address = self.my_address();
        let my_index = committee_data
            .members
            .iter()
            .position(|m| m.address == my_address)
            .context("My address not found in committee members")?;

        let dispute_channel_setup = DisputeChannelSetup::new(
            self.bitvmx_broker.clone(),
            self.config.drp_program_definition.clone(),
        );

        let requests = dispute_channel_setup.request_dispute_core_var(committee_data, my_index)?;

        self.ctx_mut().setup_channel_req = requests;

        Ok(())
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
    #[allow(clippy::too_many_arguments)]
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

const PAIRWISE_DISPUTE_KEY: &str = "PAIRWISE_DISPUTE_KEY";

/// Generates a deterministic name for a pairwise dispute key variable.
/// Both members will derive the same name regardless of who initiates.
pub fn get_dispute_pair_key_name(idx_a: usize, idx_b: usize) -> String {
    // Ensure canonical ordering (min, max) so both parties derive the same name.
    let (min_i, max_i) = if idx_a <= idx_b { (idx_a, idx_b) } else { (idx_b, idx_a) };

    format!("{PAIRWISE_DISPUTE_KEY}_{min_i}_{max_i}")
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::rc::Rc;

    use alloy_primitives::{Address as AlloyAddress, Bytes, FixedBytes, U256};
    use anyhow::anyhow;
    use bitcoin::hashes::Hash;
    use bitcoin::{PublicKey, Txid};
    use common::msg_broker::bitvmx_types::{
        CommsAddress, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, OutputType,
        ParticipantRole, SignedPublicKey,
    };
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::runtime_sync::RuntimeSync;
    use common::types::{
        Address as CommonAddress, BlockHash, BlockNumber, CommitteeId, StreamId, TxHash,
    };
    use mockall::predicate::function;
    use primitive_types::{H160, H256};
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        AllCommunicationDataReady, Committee, CommitteeMember, NewCommittee, NewPendingCommittee,
        UTXO,
    };
    use uuid::Uuid;

    use super::*;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::committee::dispute_channel_setup::DisputeChannelSetupRequest;
    use crate::flows::common::GlobalContext;
    use crate::store::MockCoordinatorStoreApi;
    use crate::types::{EventWithBlock, MemberOfCommittee, Utxo as UserUtxo};
    use crate::user_requests::ApplyToStream;

    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;
    type TestFlow =
        SetupCommitteeFlow<MockRskContractsGatewayApi, MockBitVmxBroker, MockCoordinatorStoreApi>;

    fn test_public_key(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 33];
        bytes[0] = 0x02;
        bytes[1] = seed;
        bytes[2..].fill(seed);
        PublicKey::from_slice(&bytes).unwrap_or_else(|_| {
            const COMPRESSED_G: [u8; 33] = [
                0x02, 0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce,
                0x87, 0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81,
                0x5b, 0x16, 0xf8, 0x17, 0x98,
            ];
            PublicKey::from_slice(&COMPRESSED_G).expect("valid generator public key")
        })
    }

    fn test_signed_pubkey(seed: u8, recovery_id: u8) -> SignedPublicKey {
        SignedPublicKey {
            public_key: test_public_key(seed),
            signature_r: [seed; 32],
            signature_s: [seed + 1; 32],
            recovery_id,
        }
    }

    fn test_txid(seed: u8) -> Txid {
        let bytes = [seed; 32];
        let hash = bitcoin::hashes::sha256d::Hash::from_slice(&bytes).expect("valid hash");
        Txid::from_raw_hash(hash)
    }

    fn to_u8(index: usize) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    fn to_u8_from_u32(index: u32) -> u8 {
        u8::try_from(index).expect("test index must fit in u8")
    }

    fn to_u32(index: usize) -> u32 {
        u32::try_from(index).expect("test index must fit in u32")
    }

    fn test_comms_address(port_offset: u16) -> CommsAddress {
        CommsAddress {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9_000 + port_offset),
            pubkey_hash: format!("{port_offset:064x}"),
        }
    }

    fn test_apply_to_stream(stream_id: u64) -> ApplyToStream {
        ApplyToStream {
            stream_id: StreamId::from(stream_id),
            role: ParticipantRole::Prover,
            funding_utxo: UserUtxo { value: 30_000_000 },
            speed_up_utxo: UserUtxo { value: 10_000_000 },
            advance_funds: UserUtxo { value: 2_000_000 },
        }
    }

    fn test_committee(member_address: AlloyAddress, role: u8) -> Committee {
        Committee {
            aggregatedKey: Bytes::from(vec![7u8; 32]),
            members: vec![CommitteeMember { memberAddress: member_address, role }],
            leaderAddress: member_address,
            operatorTakeIndex: U256::from(0),
            createdAt: U256::from(0),
            missingData: 0,
            missingCommunicationData: 0,
            isPending: false,
            streamId: 7,
            fundingUTXOs: vec![UTXO {
                txid: FixedBytes::<32>::from([3u8; 32]),
                outputIndex: 0,
                amount: 21_000,
            }],
        }
    }

    fn test_pending_committee_event(committee_id: u128) -> EventWithBlock<NewPendingCommittee> {
        let member_address: AlloyAddress = [1u8; 20].into();
        EventWithBlock {
            inner: NewPendingCommittee {
                committeeId: committee_id,
                _committee: test_committee(member_address, 1),
            },
            block_number: BlockNumber::from(100),
            block_hash: BlockHash::from(H256::from_low_u64_be(101)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(102)),
        }
    }

    fn test_ready_committee_event(committee_id: u128) -> EventWithBlock<NewCommittee> {
        let member_address: AlloyAddress = [2u8; 20].into();
        EventWithBlock {
            inner: NewCommittee {
                committeeId: committee_id,
                _committee: test_committee(member_address, 1),
            },
            block_number: BlockNumber::from(200),
            block_hash: BlockHash::from(H256::from_low_u64_be(201)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(202)),
        }
    }

    fn test_comm_data_ready_event(committee_id: u128) -> EventWithBlock<AllCommunicationDataReady> {
        EventWithBlock {
            inner: AllCommunicationDataReady { _committeeId: committee_id },
            block_number: BlockNumber::from(300),
            block_hash: BlockHash::from(H256::from_low_u64_be(301)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(302)),
        }
    }

    fn test_partial_utxo(index: u32) -> (Txid, u32, Option<u64>, Option<OutputType>) {
        let txid = test_txid(to_u8_from_u32(index));
        (txid, index, Some(1000 + u64::from(index)), None)
    }

    fn test_member(index: usize, role: ParticipantRole) -> MemberOfCommittee {
        MemberOfCommittee {
            address: CommonAddress::from(H160::from([to_u8(index); 20])),
            role,
            take_key: test_public_key(to_u8(index * 2)),
            dispute_key: test_public_key(to_u8(index * 2 + 1)),
            funding_utxo: test_partial_utxo(to_u32(index)),
            committee_idx: index,
        }
    }

    fn test_committee_data(committee_uuid: Uuid) -> CommitteeData {
        let committee_id = CommitteeId::from(committee_uuid.as_u128());
        let member_address: AlloyAddress = [9u8; 20].into();
        CommitteeData {
            committee_id,
            committee: test_committee(member_address, 1),
            members: vec![test_member(0, ParticipantRole::Prover)],
        }
    }

    fn create_test_flow() -> TestFlow {
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        mock_contracts.expect_my_address().return_const(CommonAddress::from(H160::from([7u8; 20])));

        SetupCommitteeFlow::new(
            Rc::new(mock_contracts),
            RuntimeSync::new().expect("runtime"),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
            Uuid::new_v4(),
            Network::Regtest,
            Rc::new(MockCoordinatorStoreApi::new()),
            CommitteeConfig::default(),
        )
    }

    #[test]
    fn test_calculate_advance_funds_value_applies_twenty_percent_buffer() {
        assert_eq!(calculate_advance_funds_value(100), 120);
        assert_eq!(calculate_advance_funds_value(101), 121);
    }

    #[test]
    fn test_get_dispute_pair_key_name_is_order_independent() {
        assert_eq!(
            get_dispute_pair_key_name(1, 4),
            get_dispute_pair_key_name(4, 1),
            "pairwise key name must be deterministic regardless of order"
        );
        assert_eq!(get_dispute_pair_key_name(2, 2), "PAIRWISE_DISPUTE_KEY_2_2");
    }

    #[test]
    fn test_signed_to_committee_public_key_normalizes_recovery_id() {
        let signed = test_signed_pubkey(4, 1);
        let result = signed_to_committee_public_key(&signed).expect("valid signed pubkey");

        assert_eq!(result.v, 28);
        assert_eq!(result.r.len(), 64);
        assert_eq!(result.s.len(), 64);
        assert_eq!(result.x.len(), 64);
        assert_eq!(result.y.len(), 64);
    }

    #[test]
    fn test_signed_to_committee_public_key_rejects_invalid_recovery_id() {
        let signed = test_signed_pubkey(4, 2);
        let err = signed_to_committee_public_key(&signed).expect_err("invalid recovery id");
        assert!(err.to_string().contains("invalid recovery_id"));
    }

    #[test]
    fn test_step_data_conversions_success_cases() {
        let apply = test_apply_to_stream(9);
        let extracted_apply = StepData::UserRequest(apply.clone()).into_user_input().unwrap();
        assert_eq!(*extracted_apply.stream_id, *apply.stream_id);

        assert_eq!(StepData::BitVmxFundingBalance(42).into_bitvmx_funding_balance().unwrap(), 42);

        let comm_info = test_comms_address(1);
        assert_eq!(
            StepData::CommInfo(comm_info.clone()).into_comms_address().unwrap().address,
            comm_info.address
        );

        let pubkey = test_public_key(3);
        assert_eq!(StepData::PublicKey(pubkey).into_pubkey().unwrap(), pubkey);

        assert_eq!(
            StepData::SignedMessage([1; 32], [2; 32], 27).into_signed_payload().unwrap(),
            ([1; 32], [2; 32], 27)
        );

        let pending_event = test_pending_committee_event(99);
        assert_eq!(
            StepData::PendingCommittee(pending_event.clone())
                .into_committee_pending()
                .unwrap()
                .inner
                .committeeId,
            pending_event.inner.committeeId
        );

        let comm_ready_event = test_comm_data_ready_event(88);
        assert_eq!(
            StepData::ReadyCommunicationData(comm_ready_event.clone())
                .into_all_comm_data_ready()
                .unwrap()
                .inner
                ._committeeId,
            comm_ready_event.inner._committeeId
        );

        let ready_event = test_ready_committee_event(77);
        assert_eq!(
            StepData::ReadyCommittee(ready_event.clone())
                .into_committee_ready()
                .unwrap()
                .inner
                .committeeId,
            ready_event.inner.committeeId
        );

        let setup_completed = Uuid::new_v4();
        assert_eq!(
            StepData::SetupCompleted(setup_completed).into_setup_completed().unwrap(),
            setup_completed
        );

        let txid = test_txid(12);
        assert_eq!(StepData::FundsSent(txid).into_funds_sent().unwrap(), txid);

        let pairwise_id = Uuid::new_v4();
        assert_eq!(
            StepData::PairwiseAggregatedKey(pairwise_id, pubkey)
                .into_pairwise_aggregated_key()
                .unwrap(),
            (pairwise_id, pubkey)
        );

        let dispute_pid = Uuid::new_v4();
        assert_eq!(
            StepData::DisputeCoreVariable(
                dispute_pid,
                OP_COSIGN_UTXOS.to_string(),
                "[]".to_string()
            )
            .into_dispute_core_variable()
            .unwrap(),
            (dispute_pid, OP_COSIGN_UTXOS.to_string(), "[]".to_string())
        );
    }

    #[test]
    fn test_step_data_conversions_fail_on_wrong_variant() {
        let err =
            StepData::PublicKey(test_public_key(1)).into_user_input().expect_err("wrong variant");
        assert!(err.to_string().contains("Expected UserRequest data"));
    }

    #[test]
    fn test_flow_context_getters_and_key_resolution() {
        let mut ctx = FlowContext {
            user_input: Some(test_apply_to_stream(11)),
            my_comm_info: Some(test_comms_address(2)),
            my_take_key_req: Some((Uuid::new_v4(), None, None, Some(test_signed_pubkey(1, 27)))),
            ..Default::default()
        };

        assert_eq!(*ctx.get_stream_id().unwrap(), 11);
        assert_eq!(ctx.get_my_comm_info().unwrap().address.port(), 9002);
        assert_eq!(ctx.get_my_take_key(&GlobalContext::new()).unwrap().recovery_id, 27);

        let global = GlobalContext::new();
        global.my_keys().set_take_key(test_signed_pubkey(9, 28));
        assert_eq!(ctx.get_my_take_key(&global).unwrap().recovery_id, 28);

        ctx.user_input = None;
        assert!(ctx.get_stream_id().is_err());
    }

    #[test]
    fn test_committee_data_helpers_and_bounds() {
        let committee_uuid = Uuid::new_v4();
        let committee_data = test_committee_data(committee_uuid);
        let member_take_key = committee_data.members[0].take_key;

        assert_eq!(committee_data.committee_uuid(), committee_uuid);
        assert_eq!(
            committee_data.get_dispute_core_pid_for_key(&member_take_key).unwrap(),
            committee_data.get_dispute_core_pid_for_index(0).unwrap()
        );
        assert!(committee_data.get_dispute_core_pid_for_index(1).is_err());
    }

    #[test]
    fn test_request_bitvmx_key_signing_sets_sign_request_id_and_sends_message() {
        let mut broker = MockBitVmxBroker::new();
        broker
            .expect_send()
            .with(function(|msg: &IncomingBitVMXApiMessages| {
                matches!(msg, IncomingBitVMXApiMessages::SignMessage(_, _, _))
            }))
            .times(1)
            .returning(|_| Ok(true));

        let mut req = Some((Uuid::new_v4(), Some(test_public_key(1)), None, None));
        TestFlow::request_bitvmx_key_signing(&mut req, &broker).expect("request signing");

        assert!(req.expect("request").2.is_some());
    }

    #[test]
    fn test_pubkey_and_core_request_matchers() {
        let pub_req_id = Uuid::new_v4();
        let sign_req_id = Uuid::new_v4();
        let pubkey_req: PubKeyReq = Some((pub_req_id, None, Some(sign_req_id), None));
        assert!(TestFlow::pubkey_request_matches(&pubkey_req, &pub_req_id));
        assert!(TestFlow::pubkey_request_matches(&pubkey_req, &sign_req_id));
        assert!(!TestFlow::pubkey_request_matches(&pubkey_req, &Uuid::new_v4()));

        let core_req_id = Uuid::new_v4();
        let committee_id = CommitteeId::from(7_u128);
        let setup_core = vec![(core_req_id, committee_id.clone(), false)];
        assert!(TestFlow::setup_core_request_matches(
            &setup_core,
            &core_req_id,
            &Ok(committee_id.clone())
        ));
        assert!(!TestFlow::setup_core_request_matches(
            &setup_core,
            &core_req_id,
            &Ok(CommitteeId::from(8_u128))
        ));
        let missing_id: Result<CommitteeId> = Err(anyhow!("committee not available"));
        assert!(!TestFlow::setup_core_request_matches(&setup_core, &core_req_id, &missing_id));
    }

    #[test]
    fn test_close_pairwise_key_request_updates_state_and_pending_flag() {
        let req_a = Uuid::new_v4();
        let req_b = Uuid::new_v4();
        let addr_a = CommonAddress::from(H160::from([1u8; 20]));
        let addr_b = CommonAddress::from(H160::from([2u8; 20]));
        let addr_c = CommonAddress::from(H160::from([3u8; 20]));

        let mut requests = vec![
            (req_a, 0, 1, vec![addr_a, addr_b], None),
            (req_b, 0, 2, vec![addr_a, addr_c], None),
        ];

        let (missing, my_idx, partner_idx) =
            TestFlow::close_pairwise_key_req(&mut requests, &req_a, test_public_key(10)).unwrap();
        assert!(missing);
        assert_eq!((my_idx, partner_idx), (0, 1));
        assert!(requests[0].4.is_some());
        assert!(requests[1].4.is_none());

        let (missing, _, _) =
            TestFlow::close_pairwise_key_req(&mut requests, &req_b, test_public_key(11)).unwrap();
        assert!(!missing);
        assert!(requests.iter().all(|r| r.4.is_some()));
    }

    #[test]
    fn test_close_pairwise_key_request_errors_for_unknown_request_id() {
        let mut requests = vec![(
            Uuid::new_v4(),
            0,
            1,
            vec![
                CommonAddress::from(H160::from([1u8; 20])),
                CommonAddress::from(H160::from([2u8; 20])),
            ],
            None,
        )];

        let err =
            TestFlow::close_pairwise_key_req(&mut requests, &Uuid::new_v4(), test_public_key(10))
                .expect_err("missing request");
        assert!(err.to_string().contains("No matching pairwise key request found"));
    }

    #[test]
    fn test_close_setup_requests_update_completion_state() {
        let req_a = Uuid::new_v4();
        let req_b = Uuid::new_v4();

        let mut setup_core_req = vec![
            (req_a, CommitteeId::from(11_u128), false),
            (req_b, CommitteeId::from(11_u128), false),
        ];
        assert!(
            TestFlow::close_setup_core_req(&mut setup_core_req, StepData::SetupCompleted(req_a))
                .unwrap()
        );
        assert!(setup_core_req[0].2);
        assert!(setup_core_req[1..].iter().any(|r| !r.2));

        let mut setup_channel_req = vec![(req_a, false), (req_b, false)];
        assert!(
            TestFlow::close_setup_channel_req(
                &mut setup_channel_req,
                StepData::SetupCompleted(req_a)
            )
            .unwrap()
        );
        assert!(setup_channel_req[0].1);
        assert!(!setup_channel_req[1].1);
    }

    #[test]
    fn test_parse_member_key_and_build_member_funding_utxo() {
        let valid_xonly =
            "79be667ef9dcbbac55a06295ce870b07029bfcd b2dce28d959f2815b16f81798".replace(' ', "");
        let member_addr: AlloyAddress = [5u8; 20].into();
        let key = TestFlow::parse_member_key(&valid_xonly, "Take", member_addr).unwrap();
        assert_eq!(key.inner.serialize()[0], 0x02);

        let invalid = "not_a_hex_key".to_string();
        assert!(TestFlow::parse_member_key(&invalid, "Take", member_addr).is_err());

        let contracts_utxo =
            UTXO { txid: FixedBytes::<32>::from([9u8; 32]), outputIndex: 3, amount: 1_500 };
        let partial = TestFlow::build_member_funding_utxo(&key, &contracts_utxo).unwrap();

        assert_eq!(partial.1, 3);
        assert_eq!(partial.2, Some(1_500));
        match partial.3 {
            Some(OutputType::SegwitPublicKey { public_key, .. }) => assert_eq!(public_key, key),
            _ => panic!("expected SegwitPublicKey output"),
        }
    }

    #[test]
    fn test_factory_creates_flow_and_restores_saved_state() {
        let mut contracts = MockRskContractsGatewayApi::new();
        contracts.expect_my_address().return_const(CommonAddress::from(H160::from([1u8; 20])));
        let contracts = Rc::new(contracts);

        let factory = SetupCommitteeFlowFactory::new(
            contracts,
            RuntimeSync::new().expect("runtime"),
            Rc::new(MockBitVmxBroker::new()),
            GlobalContext::new(),
            Network::Regtest,
            Rc::new(MockCoordinatorStoreApi::new()),
            CommitteeConfig::default(),
        );

        let internal_id = Uuid::new_v4();
        let flow = factory.create_flow(internal_id);
        assert_eq!(flow.internal_id(), internal_id);
        assert_eq!(flow.current_step(), Steps::Init);

        let saved = State { internal_id, step: Steps::Done, ctx: FlowContext::default() };
        let restored = factory.create_flow_from_saved_state(saved);
        assert_eq!(restored.internal_id(), internal_id);
        assert_eq!(restored.current_step(), Steps::Done);
    }

    #[test]
    fn test_flow_accessors_match_stream_committee_and_program_ids() {
        let mut flow = create_test_flow();
        flow.state.ctx.user_input = Some(test_apply_to_stream(55));
        flow.state.ctx.funding_balance_req = Some((Uuid::new_v4(), None));

        let target_stream = StreamId::from(55);
        assert!(flow.is_for_stream(&target_stream));
        assert!(!flow.is_for_stream(&StreamId::from(99)));

        let pending = test_pending_committee_event(1234);
        flow.state.ctx.committee_pending_ev = Some(pending.clone());
        let committee_id: CommitteeId = pending.inner.committeeId.into();
        assert!(flow.is_for_committee(&committee_id));
        assert!(!flow.is_for_committee(&CommitteeId::from(1_u128)));

        let req_id = Uuid::new_v4();
        flow.state.step = Steps::RequestDisputeChannelVars;
        flow.state.ctx.setup_channel_req = vec![DisputeChannelSetupRequest {
            dispute_core_pid: req_id,
            member_index: 0,
            op_cosign_utxos: None,
            wt_init_challenge_utxos: None,
        }];
        assert!(flow.is_waiting_for_dispute_core_variable(&req_id));
        assert!(!flow.is_waiting_for_dispute_core_variable(&Uuid::new_v4()));

        let unknown_req = Uuid::new_v4();
        assert!(!flow.is_waiting_for_bitvmx_request(&unknown_req));
    }

    #[test]
    fn test_validate_state_serialization_accepts_valid_state() {
        let state =
            State { internal_id: Uuid::new_v4(), step: Steps::Init, ctx: FlowContext::default() };
        assert!(TestFlow::validate_state_serialization(&state).is_ok());
    }
}
