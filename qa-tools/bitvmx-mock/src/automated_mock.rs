use anyhow::{Context, Result};
use bitcoin::Transaction;
use common::msg_broker::bitvmx_types::{BtcTxSPVProof, VariableTypes};
use common::msg_broker::broker::BrokerServerApi;
use common::msg_broker::{
    bitvmx_types::{
        IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, TransactionBlockchainStatus,
        TransactionStatus,
    },
    broker::{BitVmxBrokerServer, BITVMX_L2_BROKER_CLIENT_ID},
};
use log::{debug, info};
use serde::Deserialize;
use serde_json::json;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RequestPeginTempInfo {
    pub rskDestinationAddress: String,
    pub btcReimbursementPubKey: String,
    pub acceptPeginSignatureHash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrevoutData {
    pub value: u64,
    pub scriptPubKey: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PeginRequestedPayload {
    pub txid: String,
    pub amount: u64,
    pub accept_pegin_sighash: Vec<u8>,
    pub take_aggregated_key: String,
    pub operators_take_key: Vec<String>,
    pub slot_index: u64,
    pub committee_id: String,
    pub rootstock_address: String,
    pub reimbursement_pubkey: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StreamPosition {
    pub stream_id: u64,
    pub packet_number: u64,
    pub slot_id: u64,
    pub peg_status: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PeginAcceptedPayload {
    pub block_hash: String,
    pub accept_pegin_tx_hash: String,
    pub pegin_request_tx_hash: String,
    pub vout: u64,
    pub stream_position: StreamPosition,
    pub speed_up_pub_key: String,
    pub rsk_destination_address: String,
    pub rbtc_amount: String,
    pub utxo_script_pub_key: String,
}

impl fmt::Debug for AutomatedBitVmxMock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AutomatedBitVmxMock")
            .field("broker_server", &"<BitVmxBrokerServer>")
            .field("pending_requests", &"<HashMap>")
            .field("background", &"<JoinHandle>")
            .finish()
    }
}

pub struct AutomatedBitVmxMock {
    broker_server: Arc<Mutex<BitVmxBrokerServer>>,
    cancel: CancellationToken,
    started: Arc<Notify>,
    background: Mutex<Option<JoinHandle<Result<()>>>>,
    block_hash: Arc<Mutex<Option<String>>>,
    tx: Arc<Mutex<Option<Transaction>>>,
    merkle_branch_path: Arc<Mutex<Option<String>>>,
    merkle_branch_hashes: Arc<Mutex<Option<Vec<String>>>>,
    last_pegin_requested_flow_id: Arc<Mutex<Option<Uuid>>>,
    last_pegin_accepted_flow_id: Arc<Mutex<Option<Uuid>>>,
}

impl AutomatedBitVmxMock {
    pub fn new(port: u16) -> Arc<Self> {
        let server = BitVmxBrokerServer::new(port);
        Arc::new(Self {
            broker_server: Arc::new(Mutex::new(server)),
            cancel: CancellationToken::new(),
            started: Arc::new(Notify::new()),
            background: Mutex::new(None),
            block_hash: Arc::new(Mutex::new(None)),
            tx: Arc::new(Mutex::new(None)),
            merkle_branch_path: Arc::new(Mutex::new(None)),
            merkle_branch_hashes: Arc::new(Mutex::new(None)),
            last_pegin_requested_flow_id: Arc::new(Mutex::new(None)),
            last_pegin_accepted_flow_id: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let me = Arc::clone(self);
        let handle = tokio::spawn(async move { me.main_loop().await });
        *self.background.lock().unwrap() = Some(handle);
        self.started.notified().await;
        Ok(())
    }

    async fn main_loop(self: Arc<Self>) -> Result<()> {
        info!("Starting BitVMX mock server async...");
        self.started.notify_waiters();
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        loop {
            loop {
                let next = { self.broker_server.lock().unwrap().try_recv().ok().flatten() };
                if let Some((req, _sender)) = next {
                    self.handle_request(req)?;
                } else {
                    break;
                }
            }
            tokio::select! {
                _ = tick.tick() => { /* poll again */ }
                _ = self.cancel.cancelled() => {
                    info!("BitVMX mock server async task stopped");
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_request(&self, request: IncomingBitVMXApiMessages) -> Result<()> {
        match request {
            IncomingBitVMXApiMessages::Ping() => {
                debug!("Auto-responding to Ping with Pong");
                let _ = {
                    let mut srv = self.broker_server.lock().unwrap();
                    srv.send(
                        &OutgoingBitVMXApiMessages::Pong(),
                        BITVMX_L2_BROKER_CLIENT_ID,
                    )
                };
            }
            IncomingBitVMXApiMessages::SetVar(flow_id, name, VariableTypes::String(data)) => {
                println!("SetVar received: {}", name);
                match name.as_str() {
                    "PeginRequest" => {
                        println!("PeginRequest received: {}", data);
                        let payload: PeginRequestedPayload = serde_json::from_str(&data)
                            .expect("Invalid PeginRequested JSON payload");
                        println!("Pegin requested payload: {:?}", payload);
                        *self.last_pegin_requested_flow_id.lock().unwrap() = Some(flow_id);
                    }
                    "PeginAccepted" => {
                        let payload: PeginAcceptedPayload = serde_json::from_str(&data)
                            .context("Invalid PeginAccepted JSON payload")?;
                        info!("Pegin accepted payload: {:?}", payload);
                        *self.last_pegin_accepted_flow_id.lock().unwrap() = Some(flow_id);
                    }
                    _ => {
                        info!("Unhandled SetVar name: {}", name);
                    }
                }
            }
            IncomingBitVMXApiMessages::GetSPVProof(_tx_id) => {
                let bh = self
                    .block_hash
                    .lock()
                    .unwrap()
                    .clone()
                    .context("Block hash not set")?;
                let tx = self.tx.lock().unwrap().clone().context("Tx not set")?;
                let path = self
                    .merkle_branch_path
                    .lock()
                    .unwrap()
                    .clone()
                    .context("Path not set")?;
                let list = self
                    .merkle_branch_hashes
                    .lock()
                    .unwrap()
                    .clone()
                    .context("Hashes not set")?;
                let hashes: Vec<[u8; 32]> = list
                    .into_iter()
                    .map(|hex_str| {
                        let bytes =
                            hex::decode(hex_str.trim_start_matches("0x")).context("Invalid hex")?;
                        <[u8; 32]>::try_from(bytes.as_slice()).context("Invalid hash length")
                    })
                    .collect::<Result<_>>()?;
                let proof = BtcTxSPVProof {
                    block_hash: bh.clone(),
                    tx: tx.clone(),
                    merkle_branch_path: path.clone(),
                    merkle_branch_hashes: hashes,
                };
                let event = OutgoingBitVMXApiMessages::SPVProof(tx.compute_txid(), Some(proof));
                let _ = {
                    let srv = self.broker_server.lock().unwrap();
                    srv.send(&event, BITVMX_L2_BROKER_CLIENT_ID)
                };
            }
            IncomingBitVMXApiMessages::SubscribeToRskPegin() => {
                info!("Coordinator subscribed to RSK pegin events");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn get_last_pegin_requested_flow_id(&self) -> Option<String> {
        self.last_pegin_requested_flow_id
            .lock()
            .unwrap()
            .as_ref()
            .map(|id| id.to_string())
    }

    pub fn get_last_pegin_accepted_flow_id(&self) -> Option<String> {
        self.last_pegin_accepted_flow_id
            .lock()
            .unwrap()
            .as_ref()
            .map(|id| id.to_string())
    }

    pub fn trigger_pegin_found(
        &self,
        tx: Transaction,
        block_hash: String,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        *self.tx.lock().unwrap() = Some(tx.clone());
        *self.block_hash.lock().unwrap() = Some(block_hash);
        *self.merkle_branch_path.lock().unwrap() = Some(merkle_branch_path);
        *self.merkle_branch_hashes.lock().unwrap() = Some(merkle_branch_hashes);

        let tx_id = tx.compute_txid();
        let tx_status = TransactionStatus {
            tx_id,
            tx: tx.clone(),
            block_info: None,
            confirmations: 1,
            status: TransactionBlockchainStatus::Confirmed,
        };
        let event = OutgoingBitVMXApiMessages::PeginTransactionFound(tx_id, tx_status);
        self.broker_server
            .lock()
            .unwrap()
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context("Failed to trigger pegin found")?;
        Ok(())
    }

    pub fn accept_pegin(
        &self,
        accept_tx: Transaction,
        block_hash: String,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        let hashes: Vec<[u8; 32]> = merkle_branch_hashes
            .into_iter()
            .map(|hex_str| {
                let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                    .context(format!("Invalid hex string: {}", hex_str))?;
                <[u8; 32]>::try_from(bytes.as_slice())
                    .map_err(|_| anyhow::anyhow!("Invalid hash length: expected 32 bytes"))
            })
            .collect::<Result<_>>()?;
        let spv_proof = BtcTxSPVProof {
            block_hash: block_hash.clone(),
            tx: accept_tx.clone(),
            merkle_branch_path: merkle_branch_path.clone(),
            merkle_branch_hashes: hashes,
        };
        let payload = json!(spv_proof);
        let flow_id = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            "accept-pegin".to_string(),
            VariableTypes::String(payload.to_string()),
        );
        self.broker_server
            .lock()
            .unwrap()
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context("Failed to send accept-pegin Variable event")?;
        Ok(())
    }

    pub async fn stop(&self) {
        self.cancel.cancel();
        if let Some(h) = self.background.lock().unwrap().take() {
            let _ = h.await;
        }
    }
}
