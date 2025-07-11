use anyhow::{Context, Result};
use bitvmx_client::{
    program::variables::VariableTypes,
    types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages},
};
use common::msg_broker::broker::{BITVMX_L2_BROKER_CLIENT_ID, BitVmxBrokerServerApi};
use serde::{Deserialize, Serialize};
use serde_json::json;
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

    pub fn send_pegin_requested_event(
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
            "register-pegin".to_string(),
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
