use common::types::Hash256;
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
// TODO improve these structs with proper typing for their fields now that we removed the http server

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransaction {
    pub version: u32,
    pub inputs: Vec<BitcoinTransactionIn>,
    pub outputs: Vec<BitcoinTransactionOut>,
    pub lock_time: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionIn {
    pub tx_id: String,
    pub v_out: u32,
    pub sequence: u32,
    pub script_sig: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionOut {
    pub amount: u64,
    pub script_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PegInAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PegInAddressOutput {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RegisterPegInInput {
    pub block_hash: String,
    pub btc_tx: BitcoinTransaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct RegisterPegInOutput {
    pub transaction_hash: String,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RegisterPegOutInput {
    pub amount_in_wei: u64,
    pub usr_pub_key: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct RegisterPegOutOutput {
    pub transaction_hash: String,
    pub success: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct AddMemberNonceInput {
    pub hash_to_sign: Hash256,
    pub nonce: PubNonce,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct AddMemberSignatureInput {
    pub hash_to_sign: Hash256,
    pub signature: PartialSignature,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct TxSentOutput {
    pub transaction_hash: String,
    pub success: bool,
}

pub type AddMemberNonceOutput = TxSentOutput;
pub type AddMemberSignatureOutput = TxSentOutput;
pub type AcceptPegInInput = RegisterPegInInput;
pub type AcceptPegInOutput = RegisterPegInOutput;
