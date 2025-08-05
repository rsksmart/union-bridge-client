// TODO(jira) https://rsklabs.atlassian.net/browse/ub-176

use crate::types::Hash256;
use bitcoin::{Amount, BlockHash, PrivateKey, PublicKey, ScriptBuf, Transaction, Txid};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

type ProgramId = Uuid;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum IncomingBitVMXApiMessages {
    Ping(),
    SetVar(Uuid, String, VariableTypes),
    SetWitness(Uuid, String, WitnessTypes),
    GetVar(Uuid, String),
    GetWitness(Uuid, String),
    GetCommInfo(),
    GetTransaction(Uuid, Txid),
    GetTransactionInofByName(Uuid, String),
    GetHashedMessage(Uuid, String, u32, u32),
    Setup(ProgramId, String, Vec<P2PAddress>, u16),
    SubscribeToTransaction(Uuid, Txid),
    SubscribeUTXO(),
    SubscribeToRskPegin(),
    DispatchTransaction(Uuid, Transaction),
    DispatchTransactionName(Uuid, String),
    SetupKey(Uuid, Vec<P2PAddress>, u16),
    GetAggregatedPubkey(Uuid),
    GetKeyPair(Uuid),
    GenerateZKP(Uuid, Vec<u8>),
    ProofReady(Uuid),
    ExecuteZKP(),
    GetZKPExecutionResult(),
    Finalize(),
    GetSPVProof(Txid),
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
    ZKPResult(/* Add appropriate type */),
    ExecutionResult(/* Add appropriate type */),
    CommInfo(P2PAddress),
    KeyPair(Uuid, PrivateKey, PublicKey),
    Variable(Uuid, String, VariableTypes),
    Witness(Uuid, String, WitnessTypes),
    NotFound(Uuid, String),
    HashedMessage(Uuid, String, u32, u32, String),
    ProofReady(Uuid),
    ProofNotReady(Uuid),
    SPVProof(Txid, Option<BtcTxSPVProof>),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BtcTxSPVProof {
    pub block_hash: String,
    pub tx: Transaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<[u8; 32]>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BitVmxSigningInfo {
    pub protocol_name: String,
    // TODO not used for now
    pub take_aggr_key: PublicKey,
    // TODO there is a TODO on the BitVMX side suggesting it will be included, but for now we will have to store it ourselves
    #[serde(default)]
    pub hash_to_sign: Hash256,
    pub signature: PartialSignature,
    pub nonce: PubNonce,
}
