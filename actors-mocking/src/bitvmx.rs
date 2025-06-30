use anyhow::{Context, Result};
use bitvmx_client::types::IncomingBitVMXApiMessages;
use common::msg_broker::{
    broker::BrokerServerApi,
    types::{FromServer, ToServer},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

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

pub struct Executor<BS: BrokerServerApi> {
    broker_server: BS,
    consumers: HashSet<u32>,
}

impl<BS: BrokerServerApi> Executor<BS> {
    pub fn new(broker_server: BS) -> Self {
        Self {
            broker_server,
            consumers: HashSet::new(),
        }
    }

    pub fn try_recv(&mut self) -> Result<()> {
        match self.broker_server.try_recv()? {
            Some((ToServer::SubscribeMockedBitVMX, consumer_id)) => {
                println!("Status: New consumer {consumer_id} for BitVMX messages");
                self.consumers.insert(consumer_id);
            }
            Some((ToServer::UnsubscribeMockedBitVMX, consumer_id)) => {
                if self.consumers.contains(&consumer_id) {
                    println!("Status: Unsubscribing consumer {consumer_id}");
                    self.consumers.remove(&consumer_id);
                }
            }
            Some((ToServer::TemporaryPegInAddressMockedBitVMX(value), consumer_id)) => {
                println!(
                    "Received TemporaryPegInAddressMockedBitVMX from consumer {consumer_id}: {}",
                    serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|e| format!("(invalid JSON: {e})"))
                );
            }
            Some((ToServer::ToBitVMX(msg), from)) => match msg {
                IncomingBitVMXApiMessages::GenerateZKP(id, data) => {
                    println!(
                        "Received GenerateZKP from {from} with id {id} and data {:?}",
                        hex::encode(data)
                    );
                }
                IncomingBitVMXApiMessages::SetVar(uuid, name, value) => {
                    println!(
                        "Received SetVar from {from}: uuid = {uuid}, name = {name}, value = {}",
                        serde_json::to_string_pretty(&value)
                            .unwrap_or_else(|e| format!("(invalid JSON: {e})"))
                    );
                }
                _ => {
                    println!("Unexpected IncomingBitVMXApiMessages received {:?}", msg);
                }
            },
            Some((_, consumer_id)) => {
                println!(
                    "Status: Unexpected request type from consumer {consumer_id}, unsubscribing"
                );
                self.consumers.remove(&consumer_id);
            }
            None => {}
        }

        Ok(())
    }

    pub fn send_get_temporary_pegin_address_event(
        &self,
        rootstock_deposit_address: String,
        value: u64,
        btc_reimbursement_pub_key: String,
    ) -> Result<()> {
        let payload = json!({
            "rootstock_deposit_address": rootstock_deposit_address,
            "value": value,
            "btc_reimbursement_pub_key": btc_reimbursement_pub_key,
        });

        let event = FromServer::FromBitVMX("pegin-address".to_owned(), payload);

        self.notify_consumers(event)
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

        let event = FromServer::FromBitVMX("pegin-requested".to_owned(), payload);

        self.notify_consumers(event)
    }

    fn notify_consumers(&self, event: FromServer) -> Result<()> {
        for c_id in &self.consumers {
            println!(
                "Status: Notifying consumer {} about new event {:?}",
                c_id, event
            );

            self.broker_server
                .send(&event, *c_id)
                .context(format!("sending event {:?} to consumer {}", event, c_id))?;
        }

        Ok(())
    }
}
