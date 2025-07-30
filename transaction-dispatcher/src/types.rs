use bitcoin::{Transaction, TxIn, TxOut};
use common::{msg_broker::bitvmx_types::BtcTxSPVProof, types::Hash256};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-214

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
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PeginAddressOutput {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BtcTxSPVProofInput {
    pub block_hash: String,
    pub btc_tx: BitcoinTransaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct RequestPeginOutput {
    pub transaction_hash: String,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPegoutInput {
    pub amount_in_wei: u64,
    pub usr_pub_key: String,
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
pub type RequestPeginInput = BtcTxSPVProofInput;
pub type RegisterPegInInput = BtcTxSPVProofInput;
pub type AcceptPeginInput = BtcTxSPVProofInput;
pub type AcceptPeginOutput = RequestPeginOutput;
pub type RegisterPegoutInput = BtcTxSPVProofInput;
pub type RegisterPegoutOutput = TxSentOutput;
pub type RequestPegoutOutput = TxSentOutput;

impl From<TxIn> for BitcoinTransactionIn {
    fn from(input: TxIn) -> Self {
        BitcoinTransactionIn {
            tx_id: input.previous_output.txid.to_string(),
            v_out: input.previous_output.vout,
            sequence: input.sequence.0,
            script_sig: hex::encode(input.script_sig.into_bytes()),
        }
    }
}

impl From<TxOut> for BitcoinTransactionOut {
    fn from(output: TxOut) -> Self {
        BitcoinTransactionOut {
            amount: output.value.to_sat(),
            script_pub_key: hex::encode(output.script_pubkey.into_bytes()),
        }
    }
}

impl From<Transaction> for BitcoinTransaction {
    fn from(tx: Transaction) -> Self {
        BitcoinTransaction {
            version: tx.version.0 as u32,
            lock_time: tx.lock_time.to_consensus_u32(),
            inputs: tx.input.into_iter().map(Into::into).collect(),
            outputs: tx.output.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<BtcTxSPVProof> for RequestPeginInput {
    fn from(proof: BtcTxSPVProof) -> Self {
        RequestPeginInput {
            block_hash: proof.block_hash,
            btc_tx: BitcoinTransaction::from(proof.tx),
            merkle_branch_path: proof.merkle_branch_path,
            merkle_branch_hashes: proof
                .merkle_branch_hashes
                .into_iter()
                .map(hex::encode)
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetMemberPublicKeysOutput {
    pub public_keys: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApplyToStreamInput {
    pub stream_id: u8,
    pub role: u8,
    pub committee_public_keys: [CommitteePublicKey; 3], // TODO(iago) different type for Coordinator input, committee_public_keys is not required there and calculated afterwards
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CommitteePublicKey {
    pub x: String,
    pub y: String,
    pub r: String,
    pub s: String,
    pub v: u8,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct ApplyToStreamOutput {
    pub transaction_hash: String,
    pub success: bool,
}
