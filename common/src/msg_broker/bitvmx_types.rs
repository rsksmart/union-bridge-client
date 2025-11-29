#![allow(clippy::pedantic)]
#![allow(clippy::all)]

use anyhow::bail;
use bitcoin::address::NetworkUnchecked;
use bitcoin::{
    Address, Amount, BlockHash, PrivateKey, PublicKey, ScriptBuf, Transaction, Txid, XOnlyPublicKey,
};
use musig2::PubNonce;
use musig2::secp::MaybeScalar;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const ACCEPT_PEGIN_TX: &str = "ACCEPT_PEGIN_TX";

type ProgramId = Uuid;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum IncomingBitVMXApiMessages {
    Ping(),
    SetVar(Uuid, String, VariableTypes),
    SetWitness(Uuid, String, WitnessTypes),
    SetFundingUtxo(Utxo),
    GetVar(Uuid, String),
    GetWitness(Uuid, String),
    GetCommInfo(),
    GetTransaction(Uuid, Txid),
    GetTransactionInfoByName(Uuid, String),
    GetHashedMessage(Uuid, String, u32, u32),
    Setup(ProgramId, String, Vec<P2PAddress>, u16),
    SubscribeToTransaction(Uuid, Txid),
    SubscribeUTXO(),
    SubscribeToRskPegin(),
    GetSPVProof(Txid),
    DispatchTransaction(Uuid, Transaction),
    DispatchTransactionName(Uuid, String),
    SetupKey(Uuid, Vec<P2PAddress>, Option<Vec<PublicKey>>, u16),
    GetAggregatedPubkey(Uuid),
    GetKeyPair(Uuid),
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
    Pong(),
    // response for transaction get and dispatch
    Transaction(Uuid, TransactionStatus, Option<String>),
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
    CommInfo(P2PAddress),
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
        Utxo {
            txid,
            vout,
            amount,
            pub_key: *pub_key,
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputType {
    Taproot {
        value: Amount,
        internal_key: PublicKey,
        script_pubkey: ScriptBuf,
        leaves: Vec<ProtocolScript>,
    },
    SegwitPublicKey {
        value: Amount,
        script_pubkey: ScriptBuf,
        public_key: PublicKey,
    },
    SegwitScript {
        value: Amount,
        script_pubkey: ScriptBuf,
        script: ProtocolScript,
    },
    SegwitUnspendable {
        value: Amount,
        script_pubkey: ScriptBuf,
    },
    ExternalUnknown {
        script_pubkey: ScriptBuf,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolScript {
    script: ScriptBuf,
    keys: HashMap<String, ScriptKey>,
    verifying_key: Option<PublicKey>,
    sign_mode: SignMode,
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

#[derive(PartialEq, Clone, Serialize, Deserialize, Debug)]
pub struct P2PAddress {
    pub address: String,
    pub peer_id: PeerId,
}

#[derive(Clone, Hash, Default, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerId(pub String);

impl PeerId {
    pub fn from_der(public_key_der: Vec<u8>) -> Self {
        PeerId(hex::encode(public_key_der))
    }
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
    pub stream_id: u64,
    pub packet_number: u64,
    pub slot_index: usize,
    pub amount: u64,
    pub pegout_id: Vec<u8>,
    pub pegout_signature_hash: Vec<u8>,
    pub pegout_signature_message: Vec<u8>,
    pub user_pubkey: PublicKey,
    pub take_aggregated_key: PublicKey,
}

impl PegOutRequest {
    pub fn name() -> String {
        "pegout_request".to_string()
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
    pub operator_count: u32,
    pub packet_size: u32,
}

impl Committee {
    pub fn name() -> String {
        "committee".to_string()
    }

    pub fn indexes_map(&self) -> HashMap<PublicKey, usize> {
        self.members
            .iter()
            .enumerate()
            .map(|(index, member)| (member.take_key, index))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParticipantRole {
    Prover,
    Verifier,
}

impl PartialEq<ParticipantRole> for u8 {
    fn eq(&self, other: &ParticipantRole) -> bool {
        let u8_other: u8 = other.into();
        self.eq(&u8_other)
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

impl Into<u8> for &ParticipantRole {
    fn into(self) -> u8 {
        if self == &ParticipantRole::Prover {
            return 1;
        }
        2 // Verifier
    }
}

/// Data structure received from BitVMX client containing pegin acceptance information.
/// This is sent after BitVMX processes the pegin request and includes signature data
/// and sighashes needed for the operator take and operator won transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeginAcceptedMessage {
    pub committee_id: Uuid,
    pub accept_pegin_txid: Txid,
    pub accept_pegin_sighash: Vec<u8>,
    pub accept_pegin_nonce: PubNonce,
    pub accept_pegin_signature: MaybeScalar,
    pub operator_take_sighash: Vec<u8>,
    pub operator_won_sighash: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeCoreData {
    pub committee_id: Uuid,
    pub operator_index: usize,
    pub operator_utxo: PartialUtxo,
    pub operator_take_pubkey: PublicKey,
}

impl DisputeCoreData {
    pub fn name() -> String {
        "dispute_core_data".to_string()
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Destination {
    Address(String, u64),   // (address, amount in sats)
    P2WPKH(PublicKey, u64), // (pubkey, amount in sats)
    Batch(Vec<Destination>),
    P2TR(XOnlyPublicKey, Vec<ProtocolScript>, u64), // (xpubkey, tap_leaves, amount in sats)
}
