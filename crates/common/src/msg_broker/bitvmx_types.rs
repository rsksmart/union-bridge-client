#![allow(clippy::pedantic)]
#![allow(clippy::all)]
use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::{Result, bail};
use bitcoin::address::NetworkUnchecked;
use bitcoin::{
    Address, Amount, BlockHash, PrivateKey, PublicKey, ScriptBuf, Transaction, Txid, XOnlyPublicKey,
};
pub use bitvmx_emulator::decision::challenge::{ForceChallenge, ForceCondition};
pub use bitvmx_emulator::executor::utils::{
    FailConfiguration, FailExecute, FailOpcode, FailRead, FailReads, FailSelectionBits, FailWrite,
};
use musig2::PubNonce;
use musig2::secp::MaybeScalar;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;
use uuid::Uuid;

pub const ACCEPT_PEGIN_TX: &str = "ACCEPT_PEGIN_TX";
pub const OPERATOR_TAKE_TX: &str = "OPERATOR_TAKE_TX";
pub const OPERATOR_WON_TX: &str = "OPERATOR_WON_TX";
pub const RSK_PEGIN_TAG: &[u8] = b"RSK_PEGIN";
pub const REJECT_PEGIN_TX: &str = "REJECT_PEGIN_TX";
pub const PROGRAM_TYPE_REJECT_PEGIN: &str = "reject_pegin";

// DisputeChannel related constants and types
pub const OP_COSIGN_UTXOS: &str = "OP_COSIGN_UTXOS";
pub const WT_INIT_CHALLENGE_UTXOS: &str = "WT_INIT_CHALLENGE_UTXOS";
pub const PROGRAM_TYPE_DISPUTE_CHANNEL: &str = "dispute_channel";
pub const ADVANCE_FUNDS_INPUT: &str = "ADVANCE_FUNDS_INPUT";
pub const PROGRAM_TYPE_DRP: &str = "drp";

pub const PROGRAM_TYPE_FULL_PENALIZATION: &str = "full_penalization";

type ProgramId = Uuid;

/// Identifier for a BitVMX program — what `Setup` registers, what BitVMX
/// events route by, and what `dispute_core` on the BitVMX side re-derives
/// from on-chain inputs.
///
/// Wraps a `Uuid` to keep these values distinct in the type system from
/// other UUIDs floating through the codebase. Convert to/from a raw `Uuid`
/// with `From`/`Into` or `value()` only at the BitVMX message boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BitVmxProtocolId(Uuid);

impl BitVmxProtocolId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn value(&self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for BitVmxProtocolId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<BitVmxProtocolId> for Uuid {
    fn from(id: BitVmxProtocolId) -> Self {
        id.0
    }
}

impl std::fmt::Display for BitVmxProtocolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// `BitVMX` program id for the accept-pegin program, derived from the
/// committee and slot. Must match `get_accept_pegin_pid` in the `BitVMX`
/// client, which uses the same derivation when looking up the program
/// (e.g. in `dispute_core` cancellation paths).
pub fn accept_pegin_protocol_id(committee_id: Uuid, slot_index: usize) -> BitVmxProtocolId {
    derive_protocol_id(committee_id, slot_index, "accept_pegin")
}

/// `BitVMX` program id for the user-take program, derived from the
/// committee and slot. Mirrors `get_user_take_pid` in the `BitVMX` client
/// so the two sides agree on the program id even if BitVMX-side dispute
/// logic ever starts re-deriving it independently (today only examples/ do).
pub fn user_take_protocol_id(committee_id: Uuid, slot_index: usize) -> BitVmxProtocolId {
    derive_protocol_id(committee_id, slot_index, "user_take")
}

/// `BitVMX` program id for the advance-funds program, derived from the
/// committee and slot. Mirrors `get_advance_funds_pid` in the `BitVMX`
/// client so the two sides agree on the program id even if BitVMX-side
/// dispute logic ever starts re-deriving it independently.
pub fn advance_funds_protocol_id(committee_id: Uuid, slot_index: usize) -> BitVmxProtocolId {
    derive_protocol_id(committee_id, slot_index, "advance_funds")
}

fn derive_protocol_id(committee_id: Uuid, slot_index: usize, salt: &str) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(slot_index.to_be_bytes());
    hasher.update(salt);
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for a `DisputeCore` instance, derived from the
/// committee and a member's take key.
pub fn dispute_core_protocol_id(committee_id: Uuid, pubkey: &PublicKey) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(pubkey.to_bytes());
    hasher.update("dispute_core");
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for a `DisputeChannel` instance, derived from the
/// committee and the operator/watchtower index pair. Order-sensitive.
pub fn dispute_channel_protocol_id(
    committee_id: Uuid,
    op_index: usize,
    wt_index: usize,
) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(op_index.to_be_bytes());
    hasher.update(wt_index.to_be_bytes());
    hasher.update("dispute_channel");
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for a pairwise aggregated key between two committee
/// members. Symmetric: both members derive the same id regardless of
/// which side initiates.
pub fn pairwise_aggregated_key_protocol_id(
    committee_id: Uuid,
    idx_a: usize,
    idx_b: usize,
) -> BitVmxProtocolId {
    let (min_i, max_i) = if idx_a <= idx_b { (idx_a, idx_b) } else { (idx_b, idx_a) };
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(min_i.to_be_bytes());
    hasher.update(max_i.to_be_bytes());
    hasher.update("pairwise_aggregated_key");
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for the full-penalization program, derived from
/// the committee.
pub fn full_penalization_protocol_id(committee_id: Uuid) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update("full_penalization");
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for the take-aggregated-key program, derived from
/// the committee.
pub fn take_aggregated_key_protocol_id(committee_id: Uuid) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update("take_aggregated_key");
    finalize_protocol_id(hasher)
}

/// `BitVMX` program id for the dispute-aggregated-key program, derived
/// from the committee.
pub fn dispute_aggregated_key_protocol_id(committee_id: Uuid) -> BitVmxProtocolId {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update("dispute_aggregated_key");
    finalize_protocol_id(hasher)
}

fn finalize_protocol_id(hasher: Sha256) -> BitVmxProtocolId {
    let hash = hasher.finalize();
    let uuid_bytes: [u8; 16] = hash[..16].try_into().expect("SHA256 is always 32 bytes");
    BitVmxProtocolId::new(Uuid::from_bytes(uuid_bytes))
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum IncomingBitVMXApiMessages {
    Ping(Uuid),
    SetVar(Uuid, String, VariableTypes),
    SetWitness(Uuid, String, WitnessTypes),
    SetFundingUtxo(Utxo),
    GetVar(Uuid, String),
    GetWitness(Uuid, String),
    GetCommInfo(Uuid),
    GetTransaction(Uuid, Txid),
    GetTransactionInfoByName(Uuid, String),
    GetHashedMessage(Uuid, String, u32, u32),
    Setup(ProgramId, String, Vec<CommsAddress>, u16),
    SubscribeToTransaction(Uuid, Txid),
    SubscribeUTXO(Uuid),
    SubscribeToRskPegin(Option<u32>),
    GetSPVProof(Txid),
    DispatchTransaction(Uuid, Transaction),
    DispatchTransactionName(Uuid, String),
    SetupKey(Uuid, Vec<CommsAddress>, Option<Vec<PublicKey>>, u16),
    GetAggregatedPubkey(Uuid),
    GetKeyPair(Uuid),
    GetEvenPubKey(Uuid),
    GetPubKey(Uuid, bool),
    SignMessage(Uuid, Vec<u8>, PublicKey), // id, payload_to_sign, public_key_to_use
    GenerateZKP(Uuid, Vec<u8>, String),
    ProofReady(Uuid),
    GetZKPExecutionResult(Uuid),
    GetFundingAddress(Uuid),
    GetFundingBalance(Uuid),
    SendFunds(Uuid, Destination, Option<u64>),
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SignedPublicKey {
    pub public_key: PublicKey,
    pub signature_r: [u8; 32],
    pub signature_s: [u8; 32],
    pub recovery_id: u8,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum OutgoingBitVMXApiMessages {
    Pong(Uuid),
    // response for transaction get and dispatch
    Transaction(Uuid, TransactionStatus, Option<String>),
    // Represents when a transaction matching a generic output pattern is found
    OutputPatternTransactionFound(Txid, TransactionStatus, Vec<u8>),
    // Represents when pegin transactions is found
    PeginTransactionFound(Txid, TransactionStatus),
    // Represents when a spending utxo transaction is found
    SpendingUTXOTransactionFound(Uuid, Txid, u32, TransactionStatus),
    // Represents when a program is running out of funds
    SpeedUpProgramNoFunds(Txid),
    // Setup Completed,
    SetupCompleted(ProgramId),
    // Add response types for the new messages if needed
    AggregatedPubkey(Uuid, PublicKey),
    AggregatedPubkeyNotReady(Uuid),
    TransactionInfo(Uuid, String, Transaction),
    ZKPResult(Uuid, Vec<u8>, Vec<u8>),
    ExecutionResult(/* Add appropriate type */),
    CommInfo(Uuid, CommsAddress),
    KeyPair(Uuid, PrivateKey, PublicKey),
    PubKey(Uuid, PublicKey),
    SignedMessage(Uuid, [u8; 32], [u8; 32], u8), // id, signature_r, signature_s, recovery_id
    Variable(Uuid, String, VariableTypes),
    Witness(Uuid, String, WitnessTypes),
    NotFound(Uuid, String),
    HashedMessage(Uuid, String, u32, u32, String),
    ProofReady(Uuid),
    ProofNotReady(Uuid),
    ProofGenerationError(Uuid, String),
    SPVProof(Txid, Option<BtcTxSPVProof>),
    FundsSent(Uuid, Txid),
    FundingAddress(Uuid, Address<NetworkUnchecked>),
    FundingBalance(Uuid, u64),
    WalletNotReady(Uuid),
    WalletError(Uuid, String),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct TransactionStatus {
    pub tx_id: Txid,
    pub tx: Transaction,
    pub block_info: Option<BlockInfo>,
    pub confirmations: u32,
    pub status: TransactionBlockchainStatus,
}
pub type BlockHeight = u32;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    pub height: BlockHeight,
    pub hash: BlockHash,
    pub prev_hash: BlockHash,
    pub txs: Vec<Transaction>,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FullBlock {
    pub height: BlockHeight,
    pub hash: BlockHash,
    pub prev_hash: BlockHash,
    pub txs: Vec<Transaction>,
    pub orphan: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TransactionInfo {
    pub tx: Transaction,
    pub block_height: BlockHeight,
    pub block_hash: BlockHash,
    pub orphan: bool,
    pub confirmations: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum TransactionBlockchainStatus {
    // Represents a transaction that has been successfully confirmed by the network but a reorganizacion move it out of the chain.
    Orphan,
    // Represents a transaction that has been successfully confirmed by the network
    Confirmed,
    // Represents when the transaction was confirmed an amount of blocks
    Finalized,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum VariableTypes {
    Secret(Vec<u8>),
    PubKey(PublicKey),
    Utxo(PartialUtxo),
    Number(u32),
    String(String),
    Input(Vec<u8>),
}

impl Utxo {
    pub fn new(txid: Txid, vout: u32, amount: u64, pub_key: &PublicKey) -> Self {
        Utxo { txid, vout, amount, pub_key: *pub_key }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum WitnessTypes {
    Secret(Vec<u8>),
    Winternitz(WinternitzSignature),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinternitzSignature {
    hashes: Vec<WinternitzHash>,
    digits: Vec<u8>,
    message_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinternitzHash {
    hash: Vec<u8>,
}

pub type PartialUtxo = (Txid, u32, Option<u64>, Option<OutputType>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AmountType {
    Value(Amount),
    Return,
    Auto,
    Recover,
    None,
}
impl AmountType {
    pub fn is_none(&self) -> bool {
        matches!(self, AmountType::None)
    }
    pub fn is_return(&self) -> bool {
        matches!(self, AmountType::Return)
    }
    pub fn is_auto(&self) -> bool {
        matches!(self, AmountType::Auto)
    }
    pub fn is_recover(&self) -> bool {
        matches!(self, AmountType::Recover)
    }
    pub fn get_value(&self) -> Option<Amount> {
        match self {
            AmountType::Value(v) => Some(*v),
            _ => None,
        }
    }
}
impl From<u64> for AmountType {
    fn from(value: u64) -> Self {
        AmountType::Value(Amount::from_sat(value))
    }
}
impl From<Amount> for AmountType {
    fn from(value: Amount) -> Self {
        AmountType::Value(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputType {
    Taproot {
        value: AmountType,
        internal_key: PublicKey,
        script_pubkey: ScriptBuf,
        leaves: Vec<ProtocolScript>,
    },
    SegwitPublicKey {
        value: AmountType,
        script_pubkey: ScriptBuf,
        public_key: PublicKey,
    },
    SegwitScript {
        value: AmountType,
        script_pubkey: ScriptBuf,
        script: ProtocolScript,
    },
    SegwitUnspendable {
        value: AmountType,
        script_pubkey: ScriptBuf,
    },
    ExternalUnknown {
        script_pubkey: ScriptBuf,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StackItem {
    /// Schnorr signature (64 bytes +1 if non-default sighash).
    SchnorrSig { non_default_sighash: bool },
    /// DER-encoded ECDSA signature (use 73B worst case) +1 if non-default sighash.
    EcdsaSig { non_default_sighash: bool },
    /// Winternitz signature (size depends on the key type).
    WinternitzSig { size: usize },
    /// Raw item of a known length (e.g., pubkeys, data pushes).
    Raw { size: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolScript {
    script: ScriptBuf,
    keys: HashMap<String, ScriptKey>,
    verifying_key: Option<PublicKey>,
    sign_mode: SignMode,
    #[serde(default)]
    items: Vec<StackItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptKey {
    name: String,
    key_type: KeyType,
    key_position: u32,
    derivation_index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WinternitzType {
    SHA256,
    HASH160,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum KeyType {
    EcdsaKey,
    XOnlyKey,
    WinternitzKey(WinternitzType),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// Controls how the script is signed using the verifying key
pub enum SignMode {
    // No signature is required
    Skip,
    // The script is signed using the verifying key in ecdsa mode
    Single,
    // The script is signed using the verifying key in musig2 mode
    Aggregate,
}

pub type PubKeyHash = String;

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize, Debug)]
pub struct CommsAddress {
    pub address: SocketAddr,
    pub pubkey_hash: PubKeyHash,
}

/// Builds the communication data for the operator from the committee data
/// and the operator's p2p address.
///
/// `BitVMX` and the contracts agreed on:
/// - storing `pubkey_hash` as the communication key on `applyToStream`
/// - storing only the address as the communication data on `depositCommunicationData`
///   therefore `get_communication_data` does not bring everything we need, just
///   the address — this was agreed with Fairgate.
///
/// # Errors
/// Returns an error if the committee data is inconsistent or contains an
/// invalid address.
pub fn build_communication_data(
    my_p2p_address: &str,
    committee_addresses: &[String],
    committee_pubkey_hashes: &[PubKeyHash],
) -> Result<Vec<CommsAddress>> {
    if committee_addresses.len() != committee_pubkey_hashes.len() {
        bail!(
            "Inconsistent committee size: {} vs {}",
            committee_addresses.len(),
            committee_pubkey_hashes.len()
        );
    }

    let mut comms_addresses = vec![];
    for (committee_address, committee_pubkey_hash) in
        committee_addresses.iter().zip(committee_pubkey_hashes.iter())
    {
        let mut addr = committee_address.clone();
        // contracts require zeroed communication data for my own address on deposit,
        // so we have to tweak it here.
        if addr.is_empty() {
            addr = my_p2p_address.to_string();
        }

        comms_addresses.push(CommsAddress {
            address: addr.parse().map_err(|e| anyhow::anyhow!("Invalid address: {e}"))?,
            pubkey_hash: committee_pubkey_hash.clone(),
        });
    }

    info!("Built communication data: {comms_addresses:?}");

    Ok(comms_addresses)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BtcTxSPVProof {
    pub block_hash: String,
    pub tx: Transaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PegOutAccepted {
    pub committee_id: Uuid,
    pub user_take_txid: Txid,
    pub user_take_sighash: Vec<u8>,
    pub user_take_nonce: PubNonce,
    pub user_take_signature: MaybeScalar,
}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct PegOutRequest {
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub amount: u64,
    pub pegout_id: Vec<u8>,
    pub user_pubkey: PublicKey,
    pub pegout_sighash: Vec<u8>,
    pub take_aggregated_key: PublicKey,
}

impl PegOutRequest {
    pub fn name() -> &'static str {
        "pegout_request"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceFundsRequest {
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub pegout_id: Vec<u8>,
    pub fee: u64,
    pub user_pubkey: PublicKey,
    pub my_take_pubkey: PublicKey,
}

impl AdvanceFundsRequest {
    pub fn name() -> &'static str {
        "advance_funds_request"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundsAdvanceSPV {
    pub txid: Txid,
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub pegout_id: Vec<u8>,
    pub spv_proof: BtcTxSPVProof,
}

impl FundsAdvanceSPV {
    pub fn name() -> &'static str {
        "funds_advance_spv"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnionTxType {
    ReimbursementKickoff,
    OperatorTake,
    OperatorWon,
    Challenge,
    RevealInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnionSPVNotification {
    pub txid: Txid,
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub spv_proof: Option<BtcTxSPVProof>,
    pub tx_type: UnionTxType,
}

impl UnionSPVNotification {
    pub fn name() -> &'static str {
        "union_spv_notification"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Utxo {
    pub txid: Txid,
    pub vout: u32,
    pub amount: u64,
    pub pub_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberData {
    pub role: ParticipantRole,
    pub take_key: PublicKey,
    pub dispute_key: PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Committee {
    pub members: Vec<MemberData>,
    pub take_aggregated_key: PublicKey,
    pub dispute_aggregated_key: PublicKey,
    pub packet_size: u32,
    pub stream_denomination: u64,
    pub pegin_confirmations: u32,
    pub pegout_confirmations: u32,
    pub reject_pegin_confirmations: u32,
}

impl Committee {
    pub fn name() -> &'static str {
        "committee"
    }

    pub fn indexes_map(&self) -> HashMap<PublicKey, usize> {
        self.members.iter().enumerate().map(|(index, member)| (member.take_key, index)).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParticipantRole {
    Prover,
    Verifier,
}

impl From<ParticipantRole> for u8 {
    fn from(role: ParticipantRole) -> Self {
        match role {
            ParticipantRole::Prover => 1,
            ParticipantRole::Verifier => 2,
        }
    }
}

impl TryInto<ParticipantRole> for u8 {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ParticipantRole, Self::Error> {
        if self == 1 {
            return Ok(ParticipantRole::Prover);
        } else if self == 2 {
            return Ok(ParticipantRole::Verifier);
        }
        bail!("Invalid member role: {}", self)
    }
}

/// Data structure received from BitVMX client containing pegin acceptance information.
/// This is sent after BitVMX processes the pegin request and includes signature data
/// plus optional operator transaction sighashes for prover members.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeginAcceptedMessage {
    pub committee_id: Uuid,
    pub accept_pegin_txid: Txid,
    pub accept_pegin_sighash: Vec<u8>,
    pub accept_pegin_nonce: PubNonce,
    pub accept_pegin_signature: MaybeScalar,
    // Kept to stay aligned with the bitvmx-client payload shape. The examples' flow and
    // union-bridge-client do not currently consume these sighashes directly;
    // union-bridge-client resolves operator txids via TransactionInfoByName.
    pub operator_take_sighash: Option<Vec<u8>>,
    // Same as above, but for operator_won_txid.
    pub operator_won_sighash: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectPeginData {
    pub committee_id: Uuid,
    pub member_index: usize,
    pub txid: Txid,
}

impl RejectPeginData {
    pub fn name() -> &'static str {
        "reject_pegin_data"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeCoreData {
    pub committee_id: Uuid,
    pub member_index: usize,
    pub funding_utxo: PartialUtxo,
}

impl DisputeCoreData {
    pub fn name() -> &'static str {
        "dispute_core_data"
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Destination {
    Address(String, u64),   // (address, amount in sats)
    P2WPKH(PublicKey, u64), // (pubkey, amount in sats)
    Batch(Vec<Destination>),
    P2TR(XOnlyPublicKey, Vec<ProtocolScript>, u64), // (xpubkey, tap_leaves, amount in sats)
}

/// Global UUID used to store union-wide settings in BitVMX.
/// This is a fixed UUID derived from the string "UNION_BRIDGE-000".
pub const GLOBAL_SETTINGS_UUID: Uuid = Uuid::from_bytes(*b"UNION_BRIDGE-000");

/// Per-stream timelock settings for Bitcoin transactions.
/// These values are in Bitcoin block counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamSettings {
    /// Short timelock for quick dispute resolution
    pub short_timelock: u16,
    /// Long timelock for extended dispute periods
    pub long_timelock: u16,
    /// Timelock for operator won transactions
    pub op_won_timelock: u16,
    /// Timelock for claim gate
    pub claim_gate_timelock: u16,
    /// Timelock for input not revealed
    pub input_not_revealed_timelock: u16,
    /// Timelock for operator no cosign
    pub op_no_cosign_timelock: u16,
    /// Timelock for watchtower no challenge
    pub wt_no_challenge_timelock: u16,
    /// Timelock for request pegin
    pub request_pegin_timelock: u16,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            short_timelock: 6,
            long_timelock: 12,
            op_won_timelock: 150,
            claim_gate_timelock: 6,
            input_not_revealed_timelock: 8,
            op_no_cosign_timelock: 12,
            wt_no_challenge_timelock: 12,
            request_pegin_timelock: 12,
        }
    }
}

/// Global union settings mapping stream denominations to their timelock configurations.
/// Sent to BitVMX before dispute_core setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnionSettings {
    /// Map of stream_denomination -> StreamSettings
    pub settings: HashMap<u64, StreamSettings>,
}

impl UnionSettings {
    pub fn name() -> &'static str {
        "union_settings"
    }

    /// Create default settings with entries for stream denominations.
    /// Keys are enum indexes matching StreamDenomination in contracts:
    /// 0 = 0.001 BTC, 1 = 0.01 BTC, 2 = 0.1 BTC, 3 = 1 BTC, 4 = 10 BTC
    pub fn with_defaults() -> Self {
        let mut settings = HashMap::new();
        for i in 0..5u64 {
            settings.insert(i, StreamSettings::default());
        }
        Self { settings }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperatorChallengeResult {
    OperatorTake,
    OperatorWon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReimbursementResult {
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub txid: Txid,
    pub challenge_result: OperatorChallengeResult,
}

impl ReimbursementResult {
    pub fn name() -> &'static str {
        "reimbursement_result"
    }
}

/// Data sent to BitVMX when advance funds are registered on RSK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceFundsRegistered {
    pub committee_id: Uuid,
    pub slot_index: usize,
    pub txid: Txid,
    pub pegout_id: Vec<u8>,
    pub operator_pubkey: PublicKey,
}

impl AdvanceFundsRegistered {
    pub fn name(slot_index: usize) -> String {
        format!("ADVANCED_FUNDS_{slot_index}")
    }
}

/// Holds the UTXOs required for a watchtower to initiate a challenge.
/// These are provided by the DisputeCore protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WtInitChallengeUtxos {
    pub wt_stopper: PartialUtxo,
    pub op_stopper: PartialUtxo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullPenalizationData {
    pub committee_id: Uuid,
}

impl FullPenalizationData {
    pub fn name() -> String {
        "full_penalization_data".to_string()
    }
}
