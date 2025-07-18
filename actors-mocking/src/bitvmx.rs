use anyhow::{Context, Result};
use bitcoin::{
    Amount, OutPoint, Transaction, TxIn, TxOut, Txid, absolute::LockTime,
    blockdata::script::ScriptBuf, hashes::Hash, transaction::Version,
};
use common::msg_broker::{
    bitvmx_types::{
        BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
        TransactionBlockchainStatus, TransactionStatus, VariableTypes,
    },
    broker::{BITVMX_L2_BROKER_CLIENT_ID, BitVmxBrokerServerApi},
};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::TryFrom;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct BtcTx {
    pub version: u32,
    pub outputs: Vec<Output>,
    pub inputs: Vec<Input>,
    pub lock_time: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Output {
    pub amount: u64,
    pub script_pub_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Input {
    pub tx_id: String,
    pub v_out: u32,
    pub sequence: u32,
    pub script_sig: String,
}

impl TryFrom<BtcTx> for Transaction {
    type Error = anyhow::Error;

    fn try_from(tx: BtcTx) -> Result<Self> {
        let inputs = tx
            .inputs
            .into_iter()
            .map(|input| {
                let txid_str = input.tx_id.trim_start_matches("0x");
                let txid_bytes =
                    <[u8; 32]>::from_hex(txid_str).context("Failed to decode tx_id from hex")?;
                let txid = Txid::from_byte_array(txid_bytes);

                let script_sig_bytes = hex::decode(input.script_sig.trim_start_matches("0x"))
                    .context("Failed to decode script_sig")?;

                Ok(TxIn {
                    previous_output: OutPoint {
                        txid,
                        vout: input.v_out,
                    },
                    script_sig: ScriptBuf::from_bytes(script_sig_bytes),
                    sequence: bitcoin::Sequence::from_consensus(input.sequence),
                    witness: bitcoin::Witness::default(), // Ignored in BtcTx
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let outputs = tx
            .outputs
            .into_iter()
            .map(|output| {
                let script_bytes = hex::decode(output.script_pub_key.trim_start_matches("0x"))
                    .context("Failed to decode script_pub_key")?;
                Ok(TxOut {
                    value: Amount::from_sat(output.amount),
                    script_pubkey: ScriptBuf::from_bytes(script_bytes),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Transaction {
            version: Version(tx.version as i32),
            lock_time: LockTime::from_consensus(tx.lock_time),
            input: inputs,
            output: outputs,
        })
    }
}

pub struct Executor<BS: BitVmxBrokerServerApi> {
    broker_server: BS,
}

impl<BS: BitVmxBrokerServerApi> Executor<BS> {
    pub fn new(broker_server: BS) -> Self {
        Self { broker_server }
    }

    pub fn try_recv(&mut self) -> Result<()> {
        match self.broker_server.try_recv()? {
            Some((IncomingBitVMXApiMessages::Ping(), from)) => {
                // println!("Received Ping from {from} at {}", Self::reception_time());

                self.broker_server
                    .send(&OutgoingBitVMXApiMessages::Pong(), from)
                    .context("Failed to send Pong response")?;
            }
            Some((IncomingBitVMXApiMessages::SubscribeToRskPegin(), from)) => {
                println!(
                    "Received message 'SubscribeToRskPegin' from client '{}'",
                    from
                );
            }
            Some((IncomingBitVMXApiMessages::GetSPVProof(tx_id), from)) => {
                println!(
                    "Received message 'GetSPVProof' from client '{}': tx_id = {}",
                    from, tx_id
                );
            }
            Some((IncomingBitVMXApiMessages::SetVar(uuid, name, value), from)) => {
                println!(
                    "Received SetVar from {from}: uuid = {uuid}, name = {name}, value = {}",
                    serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|e| format!("(invalid JSON: {e})"))
                );
            }
            Some((IncomingBitVMXApiMessages::GenerateZKP(id, data), from)) => {
                println!(
                    "Received GenerateZKP from {from} with id {id} and data {:?} at {}",
                    hex::encode(data),
                    Self::reception_time()
                );
            }
            Some((msg, _from)) => {
                println!(
                    "Unexpected IncomingBitVMXApiMessages received {:?} at {}",
                    msg,
                    Self::reception_time()
                );
            }
            None => {
                // No message received
            }
        }

        Ok(())
    }

    pub fn send_pegin_transaction_found(&self, tx: BtcTx) -> Result<()> {
        let tx: Transaction = tx.try_into()?;
        let tx_id = tx.compute_txid();
        let tx_status = TransactionStatus {
            tx_id,
            tx,
            block_info: None,
            confirmations: 1,
            status: TransactionBlockchainStatus::Confirmed,
        };

        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, tx_status);

        return self
            .broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ));
    }

    pub fn send_pegin_requested_event(
        &self,
        block_hash: String,
        tx: BtcTx,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let tx: Transaction = tx.try_into()?;
        let spv_proof = BtcTxSPVProof {
            block_hash,
            tx: tx.clone(),
            merkle_branch_path,
            merkle_branch_hashes: merkle_branch_hashes
                .into_iter()
                .map(|h| {
                    let bytes = hex::decode(h.trim_start_matches("0x"))
                        .expect("Invalid hex in merkle_branch_hashes");
                    <[u8; 32]>::try_from(bytes.as_slice()).expect("Merkle hash must be 32 bytes")
                })
                .collect(),
        };

        let event = OutgoingBitVMXApiMessages::SPVProof(tx.compute_txid(), Some(spv_proof));

        return self
            .broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ));
    }

    pub fn send_pegin_accepted_event(
        &self,
        block_hash: String,
        btc_tx: BtcTx,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let payload = json!({
            "block_hash": block_hash,
            "btc_tx": btc_tx,
            "merkle_branch_path": merkle_branch_path,
            "merkle_branch_hashes": merkle_branch_hashes,
        });

        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "accept-pegin".to_string(),
            VariableTypes::String(payload.to_string()),
        );

        return self
            .broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ));
    }

    fn reception_time() -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
    }
}
