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
use log::{debug, info, warn};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio::time::Duration;
use uuid::Uuid;

pub struct AutomatedBitVmxMock {
    port: u16,
    pub broker_server: Arc<Mutex<BitVmxBrokerServer>>,
    pending_requests: Arc<Mutex<HashMap<String, IncomingBitVMXApiMessages>>>,
    running: Arc<Mutex<bool>>,
    background_handle: Option<JoinHandle<Result<()>>>,
    block_hash: Arc<Mutex<Option<String>>>,
    tx: Arc<Mutex<Option<Transaction>>>,
    merkle_branch_path: Arc<Mutex<Option<String>>>,
    merkle_branch_hashes: Arc<Mutex<Option<Vec<String>>>>,
    last_pegin_requested_flow_id: Arc<Mutex<Option<Uuid>>>,
    last_pegin_accepted_flow_id: Arc<Mutex<Option<Uuid>>>,
}

impl AutomatedBitVmxMock {
    pub fn set_running(&self, running: Arc<Mutex<bool>>) {
        info!("Setting AutomatedBitVmxMock running state");
        *self.running.lock().unwrap() = *running.lock().unwrap();
    }
}

impl AutomatedBitVmxMock {
    pub fn get_running(&self) -> Arc<Mutex<bool>> {
        Arc::clone(&self.running)
    }
}

impl Default for AutomatedBitVmxMock {
    fn default() -> Self {
        Self::new(8547)
    }
}

impl std::fmt::Debug for AutomatedBitVmxMock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomatedBitVmxMock")
            .field("pending_requests", &self.pending_requests)
            .field("running", &self.running)
            .field("broker_server", &"<BitVmxBrokerServer>")
            .finish()
    }
}

impl AutomatedBitVmxMock {
    pub fn new(port: u16) -> Self {
        info!("Creating AutomatedBitVmxMock on port {}", port);
        Self {
            port,
            broker_server: Arc::new(Mutex::new(BitVmxBrokerServer::new(port))),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(false)),
            background_handle: None,
            block_hash: Arc::new(Mutex::new(None)),
            tx: Arc::new(Mutex::new(None)),
            merkle_branch_path: Arc::new(Mutex::new(None)),
            merkle_branch_hashes: Arc::new(Mutex::new(None)),
            last_pegin_requested_flow_id: Arc::new(Mutex::new(None)),
            last_pegin_accepted_flow_id: Arc::new(Mutex::new(None)),
        }
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

    pub fn run(&mut self) -> Result<()> {
        info!("Starting BitVMX mock server async...");
        *self.running.lock().unwrap() = true;

        let running = Arc::clone(&self.running);
        let server = Arc::clone(&self.broker_server);
        let block_hash = Arc::clone(&self.block_hash);
        let tx = Arc::clone(&self.tx);
        let merkle_branch_path = Arc::clone(&self.merkle_branch_path);
        let merkle_branch_hashes = Arc::clone(&self.merkle_branch_hashes);
        let last_pegin_requested_flow_id = Arc::clone(&self.last_pegin_requested_flow_id);
        let last_pegin_accepted_flow_id = Arc::clone(&self.last_pegin_accepted_flow_id);

        let handle = tokio::spawn({
            async move {
                while *running.lock().unwrap() {
                    // receive
                    if let Ok(Some((request, _sender))) = {
                        let mut srv = server.lock().unwrap();
                        srv.try_recv()
                    } {
                        info!("Received request from coordinator: {:?}", request);

                        match request {
                            IncomingBitVMXApiMessages::Ping() => {
                                debug!("Auto-responding to Ping with Pong");
                                let _ = {
                                    let mut srv = server.lock().unwrap();
                                    srv.send(
                                        &OutgoingBitVMXApiMessages::Pong(),
                                        BITVMX_L2_BROKER_CLIENT_ID,
                                    )
                                };
                            }

                            IncomingBitVMXApiMessages::SetVar(
                                flow_id,
                                name,
                                VariableTypes::String(data),
                            ) => {
                                match name.as_str() {
                                    "PeginRequested" => {
                                        info!("SetVar PeginRequested (flow {}): {}", flow_id, data);
                                        // optionally: store for later assertions
                                        *last_pegin_requested_flow_id.lock().unwrap() =
                                            Some(flow_id);
                                    }
                                    "PeginAccepted" => {
                                        info!("SetVar PeginAccepted (flow {}): {}", flow_id, data);
                                        // optionally: store for later assertions
                                        // *self.last_pegin_accepted.lock().unwrap() = Some((flow_id, data.clone()));
                                        *last_pegin_accepted_flow_id.lock().unwrap() =
                                            Some(flow_id);
                                    }
                                    _ => {
                                        debug!("SetVar {} (flow {}): {}", name, flow_id, data);
                                    }
                                }
                            }

                            IncomingBitVMXApiMessages::GetSPVProof(tx_id) => {
                                info!("Auto-responding to GetSPVProof for tx_id: {}", tx_id);
                                info!("My block_hash: {:?}", block_hash.lock().unwrap());
                                let bh = block_hash
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .expect("Block hash not set")
                                    .clone();
                                let txobj =
                                    tx.lock().unwrap().as_ref().expect("Tx not set").clone();
                                let path = merkle_branch_path
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .expect("Path not set")
                                    .clone();
                                let list = merkle_branch_hashes
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .expect("Hashes not set")
                                    .clone();
                                let hashes: Vec<[u8; 32]> = list
                                    .into_iter()
                                    .map(|hex_str| {
                                        let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                                            .expect("Invalid hex");
                                        <[u8; 32]>::try_from(bytes.as_slice()).unwrap()
                                    })
                                    .collect();

                                let proof = {
                                    BtcTxSPVProof {
                                        block_hash: bh.clone(),
                                        tx: txobj.clone(),
                                        merkle_branch_path: path.clone(),
                                        merkle_branch_hashes: hashes,
                                    }
                                };
                                let event = OutgoingBitVMXApiMessages::SPVProof(
                                    txobj.clone().compute_txid(),
                                    Some(proof),
                                );
                                let _ = {
                                    let srv = server.lock().unwrap();
                                    srv.send(&event, BITVMX_L2_BROKER_CLIENT_ID)
                                };
                                info!(
                                    "Sent auto SPV proof for computed tx id: {}",
                                    txobj.clone().compute_txid()
                                );
                                info!("Sent auto SPV proof for tx_id: {}", tx_id);
                            }
                            IncomingBitVMXApiMessages::SubscribeToRskPegin() => {
                                info!("Coordinator subscribed to RSK pegin events");
                            }
                            _ => {
                                debug!("Ignoring message {:?}", request);
                            }
                        }
                    }
                    sleep(Duration::from_millis(10)).await;
                }
                info!("BitVMX mock server async task stopped");
                Ok(())
            }
        });
        self.background_handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) {
        info!("Stopping AutomatedBitVmxMock...");
        *self.running.lock().unwrap() = false;

        if let Some(handle) = self.background_handle.take() {
            handle.abort();
            sleep(Duration::from_millis(1000)).await;
        }
        info!("AutomatedBitVmxMock stopped");
    }

    pub fn trigger_pegin_found(
        &mut self,
        tx: Transaction,
        block_hash: String,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        *self.tx.lock().unwrap() = Some(tx.clone());
        *self.block_hash.lock().unwrap() = Some(block_hash);
        *self.merkle_branch_path.lock().unwrap() = Some(merkle_branch_path);
        *self.merkle_branch_hashes.lock().unwrap() = Some(merkle_branch_hashes); // Assuming empty for now, can be set later
        info!(
            "Triggering PeginTransactionFound for tx: {}",
            tx.compute_txid()
        );
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
        &mut self,
        accept_tx: Transaction,
        block_hash: String,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    ) -> Result<()> {
        info!(
            "Triggering accept-pegin for tx: {}",
            accept_tx.compute_txid()
        );

        // Convert hex strings to [u8; 32] arrays for merkle branch hashes
        let hashes: Vec<[u8; 32]> = merkle_branch_hashes
            .into_iter()
            .map(|hex_str| {
                let bytes = hex::decode(hex_str.trim_start_matches("0x"))
                    .context(format!("Invalid hex string: {}", hex_str))?;
                <[u8; 32]>::try_from(bytes.as_slice())
                    .map_err(|_| anyhow::anyhow!("Invalid hash length: expected 32 bytes"))
            })
            .collect::<Result<Vec<_>>>()?;

        // Build the SPV proof for the accept transaction
        let spv_proof = BtcTxSPVProof {
            block_hash: block_hash.clone(),
            tx: accept_tx.clone(),
            merkle_branch_path: merkle_branch_path.clone(),
            merkle_branch_hashes: hashes,
        };

        // Create the payload for the accept-pegin Variable event
        let payload = json!(spv_proof);

        // Generate a flow ID for this accept-pegin event
        let flow_id = uuid::Uuid::new_v4();

        info!(
            "Sending accept-pegin Variable event with flow_id: {} for tx: {}",
            flow_id,
            accept_tx.compute_txid()
        );

        // Create the accept-pegin Variable event
        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            "accept-pegin".to_string(),
            VariableTypes::String(payload.to_string()),
        );

        // Send the event to the coordinator
        self.broker_server
            .lock()
            .unwrap()
            .send(&event, BITVMX_L2_BROKER_CLIENT_ID)
            .context("Failed to send accept-pegin Variable event")?;

        info!(
            "Successfully sent accept-pegin event for tx: {}",
            accept_tx.compute_txid()
        );

        Ok(())
    }
}
