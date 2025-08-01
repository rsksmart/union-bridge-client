use anyhow::{Context, Result};
use bitcoin::{PublicKey, Transaction};
use common::msg_broker::{
    bitvmx_types::{
        BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
        TransactionBlockchainStatus, TransactionStatus, VariableTypes,
    },
    broker::{BITVMX_L2_BROKER_CLIENT_ID, BitVmxBrokerServerApi},
};
use common::types::Hash256;
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BitVmxSigningInfo {
    pub protocol_name: String,
    // TODO there not used for now
    pub take_aggr_key: PublicKey,
    // TODO there is a TODO on the BitVMX side suggesting it will be included, but for now we will have to store it ourselves
    #[serde(default)]
    pub hash_to_sign: Hash256,
    pub signature: PartialSignature,
    pub nonce: PubNonce,
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
            Some((IncomingBitVMXApiMessages::GenerateZKP(id, data), from)) => {
                println!(
                    "Received GenerateZKP from {from} with id {id} and data {:?} at {}",
                    hex::encode(data),
                    Self::reception_time()
                );
            }
            Some((IncomingBitVMXApiMessages::Ping(), from)) => {
                println!("Received Ping from {from} at {}", Self::reception_time());

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
            Some((
                IncomingBitVMXApiMessages::Setup(program_id, program_name, addresses, port),
                from,
            )) => {
                println!(
                    "Received Setup from {from}: program_id = {program_id}, program_name = {program_name}, addresses = {:?}, port = {port} at {}",
                    addresses,
                    Self::reception_time()
                );
            }
            Some((
                IncomingBitVMXApiMessages::DispatchTransactionName(uuid, transaction_name),
                from,
            )) => {
                println!(
                    "Received DispatchTransactionName from {from}: uuid = {uuid}, transaction_name = {transaction_name} at {}",
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

    pub fn send_pegin_transaction_found(&self, tx: Transaction) -> Result<()> {
        let tx_id = tx.compute_txid();
        let tx_status = TransactionStatus {
            tx_id,
            tx,
            block_info: None,
            confirmations: 1,
            status: TransactionBlockchainStatus::Confirmed,
        };

        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, tx_status);

        self.broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ))
    }

    pub fn send_pegin_requested_event(
        &self,
        block_hash: String,
        tx: Transaction,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let spv_proof =
            Self::build_spv_proof(block_hash, &tx, merkle_branch_path, merkle_branch_hashes)?;

        let event = OutgoingBitVMXApiMessages::SPVProof(tx.compute_txid(), Some(spv_proof));

        self.broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ))
    }

    pub fn send_pegin_accepted_event(
        &self,
        block_hash: String,
        tx: Transaction,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let spv_proof =
            Self::build_spv_proof(block_hash, &tx, merkle_branch_path, merkle_branch_hashes)?;

        let payload = json!(spv_proof);

        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "accept-pegin".to_string(),
            VariableTypes::String(payload.to_string()),
        );

        self.broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ))
    }

    pub fn send_pegout_requested_event(
        &self,
        amount_in_wei: u64,
        usr_pub_key: String,
    ) -> Result<()> {
        let payload = json!({
            "amount_in_wei": amount_in_wei,
            "usr_pub_key": usr_pub_key,
        });

        let uuid = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "request-pegout".to_string(),
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

    pub fn send_register_pegout_event(
        &self,
        block_hash: String,
        tx: Transaction,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let spv_proof = BtcTxSPVProof {
            block_hash,
            tx: tx.clone(),
            merkle_branch_path,
            merkle_branch_hashes: merkle_branch_hashes
                .into_iter()
                .map(|h| {
                    let bytes = hex::decode(h.trim_start_matches("0x"))
                        .expect(&format!("Invalid hex in merkle_branch_hashes: {}", h));
                    <[u8; 32]>::try_from(bytes.as_slice()).expect(&format!(
                        "Merkle hash must be 32 bytes, but received {} bytes",
                        bytes.len()
                    ))
                })
                .collect(),
        };

        let event = OutgoingBitVMXApiMessages::SPVProof(tx.compute_txid(), Some(spv_proof));

        self.broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ))
    }

    pub fn send_signature_event(&self, bit_vmx_signing_info: BitVmxSigningInfo) -> Result<()> {
        let payload = json!({
            "take_aggr_key": bit_vmx_signing_info.take_aggr_key,
            "hash_to_sign": bit_vmx_signing_info.hash_to_sign,
            "signature": bit_vmx_signing_info.signature,
            "nonce": bit_vmx_signing_info.nonce,
        });
        let uuid = Uuid::parse_str(&bit_vmx_signing_info.protocol_name)?;
        let event = OutgoingBitVMXApiMessages::Variable(
            uuid,
            "signing_info".to_string(),
            VariableTypes::String(payload.to_string()),
        );

        self.broker_server
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context(format!(
                "sending event {:?} to consumer {}",
                event, BITVMX_L2_BROKER_CLIENT_ID
            ))
    }

    fn build_spv_proof(
        block_hash: String,
        tx: &Transaction,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<BtcTxSPVProof> {
        let merkle_branch_hashes = merkle_branch_hashes
            .into_iter()
            .map(|h| {
                let bytes = hex::decode(h.trim_start_matches("0x")).map_err(|e| {
                    anyhow::anyhow!("Invalid hex in merkle_branch_hashes {}: {}", h, e)
                })?;
                <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
                    anyhow::anyhow!(
                        "Merkle hash must be 32 bytes, but received {} bytes",
                        bytes.len()
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(BtcTxSPVProof {
            block_hash,
            tx: tx.clone(),
            merkle_branch_path,
            merkle_branch_hashes,
        })
    }

    fn reception_time() -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
    }
}
