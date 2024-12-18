use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, SubscriptionItem};
use anyhow::{anyhow, bail, Ok, Result};
use futures_util::StreamExt;
use log::{debug, error, info, trace, warn};
use monitor::store::CachedKeyValueStore;
use monitor::types::{RskBlock, RskRpcBlock};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;
use tokio::time::{self, Duration};

// TODO use keys enum
const LAST_INDEXED_BLOCK: &'static str = "LAST_INDEXED_BLOCK";
const LAST_CONNECTED_BLOCK: &'static str = "LAST_CONNECTED_BLOCK";

#[derive(Clone)]
struct ShutdownFlag {
    flag: Arc<AtomicBool>,
}

impl ShutdownFlag {
    pub fn init() -> Self {
        ShutdownFlag {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request_shutdown(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let shutdown_flag = Arc::new(ShutdownFlag::init());
    let shutdown_flag_clone = Arc::clone(&shutdown_flag);

    let store = CachedKeyValueStore::new("/Users/illuque/tmp/").expect("Failed to create CachedKeyValueStore");

    let main_task = tokio::spawn(async move {
        // after boot, we do a backward_sync to catch up with the latest block
        if let Err(e) = backward_sync(&shutdown_flag_clone, &store).await {
            error!("Unrecoverable error in backward_sync_task: {:?}", e);
        }

        if shutdown_flag_clone.is_requested() {
            info!("Stopping main_task requested and skipping block_subscription_task...");
            return;
        }

        if let Err(e) = subscribe_blocks_loop(&shutdown_flag_clone, &store).await {
            error!("Unrecoverable error in block_subscription_task: {:?}", e);
        }
    });

    let control_task = tokio::spawn(async move {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        shutdown_flag.request_shutdown();

        info!("Shutdown requested! Waiting for main_task to finish...");

        // TODO timeout configurable
        let timeout_result = time::timeout(Duration::from_secs(60), main_task).await;
        if timeout_result.is_err() {
            warn!("main_task did not complete in time, forcefully cancelling...");
        } else {
            info!("main_task completed within the grace period.");
        }
    });

    control_task.await?;

    log::logger().flush();

    Ok(())
}

async fn subscribe_blocks_loop(
    shutdown_flag: &ShutdownFlag, // TODO instead of receiving this, try to receive the ws subscription and close it on ctrl+c, that should stop the loop also
    store: &CachedKeyValueStore,
) -> Result<()> {
    // TODO (make resilient, see list below)
    // 1) add last_ping to check if connection is still alive
    // 2) add exponential backoff
    // 3) add reconnect
    // 4) add backup servers
    // 5) everything configurable
    // Replace with your WebSocket provider URL

    let rpc_url = "wss://public-node.testnet.rsk.co/websocket";
    let ws = WsConnect::new(rpc_url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;
    let sub = provider.subscribe_blocks().await?;
    let mut stream = sub.into_any_stream();

    println!("Subscribed to new block headers");

    // some last second gap backward_sync in case while this task was starting we missed some block
    backward_sync(shutdown_flag, store).await?;

    info!("Start receiving blocks from eth_subscribe...");

    let subscription_result: Result<()> = async {
        let mut last_received_block_opt: Option<RskBlock> = None;

        while !shutdown_flag.is_requested() {
            let header = match stream.next().await {
                Some(header) => header,
                None => {
                    error!("eth_subscribe stream ended, stopping subscribe_blocks");
                    break;
                }
            };

            let new_block_header_raw = match header {
                SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
                _ => {
                    bail!("Unexpected SubscriptionItem: {:?}", header);
                }
            };

            let new_block_header: Value = serde_json::from_str(&*new_block_header_raw)?;
            let new_block_hash = new_block_header["hash"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing hash field"))?;

            // TODO extract to web3 connector abstraction class
            let new_block = fetch_block_data(&provider, Some(&new_block_hash), None).await?;
            info!(
                "Processing block {} ({}) on subscription",
                new_block.number(),
                new_block.hash()
            );

            check_reorg(&last_received_block_opt, &new_block);

            // TODO move to store abstraction class
            // TODO create keys enum
            let block_key = &format!("block_{}", new_block.number());
            store.save_block(block_key, &new_block)?;
            // TODO create keys enum
            store.save_block(LAST_INDEXED_BLOCK, &new_block)?;
            last_received_block_opt = Some(new_block);
        }

        if let Some(last_received_block) = last_received_block_opt {
            // update connection point to best block
            store.save_block(LAST_CONNECTED_BLOCK, &last_received_block)?;
        }

        info!("Stopping subscribe_blocks requested...");

        Ok(())
    }
    .await;

    // TODO do this close outside if reused by http calls
    // TODO check why sometimes I get "WS connection error: Closed" (probably still being reused by some task)
    // TODO run eth_unsubscribe with subscription id
    // TODO handle reconnect on error

    drop(provider);

    subscription_result
}

fn check_reorg(last_received_block_opt: &Option<RskBlock>, new_block: &RskBlock) {
    if last_received_block_opt.is_none() {
        trace!("No LAST_INDEXED_BLOCK found, first run?");
        return;
    }

    let last_received_block = last_received_block_opt.as_ref().unwrap();
    if new_block.number() <= last_received_block.number() {
        warn!(
            "Reorg detected between {} ({}) and {} ({})",
            last_received_block.number(),
            last_received_block.hash(),
            new_block.number(),
            new_block.hash()
        );
    }
}

async fn fetch_block_data(
    provider: &RootProvider<PubSubFrontend>,
    block_hash: Option<&str>,
    block_number: Option<&u64>, // TODO improve with wrapper type like BlockId
) -> Result<RskBlock> {
    if block_hash.is_none() && block_number.is_none() {
        bail!("Either block_hash or block_number_or_ref must be provided");
    }

    if block_hash.is_some() && block_number.is_some() {
        bail!("Only one of block_hash or block_number_or_ref must be provided");
    }

    let (method, block_id) = if block_hash.is_some() {
        ("eth_getBlockByHash", block_hash.unwrap().to_string())
    } else {
        (
            "eth_getBlockByNumber",
            format!("0x{:x}", block_number.unwrap()),
        )
    };

    let response: Value = provider
        .client()
        .request(method, vec![json!(block_id), json!(false)])
        .await?;

    // TODO resilience when response is not a block (ie not found)

    let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
    let rsk_block: RskBlock = RskBlock::from(rpc_block);

    Ok(rsk_block)
}

async fn backward_sync(shutdown_flag: &ShutdownFlag, store: &CachedKeyValueStore) -> Result<()> {
    // TODO improve this logic passing last_connected_block as parameter
    // we have to reach LAST_CONNECTED_BLOCK:
    // - if LAST_CONNECTED_BLOCK not present (first time app is run) we have to reach LAST_INDEXED_BLOCK
    // - if LAST_INDEXED_BLOCK not present either (first time app is run), we skip the sync
    let last_connected_block: Option<RskBlock> = store
        .get_block(LAST_CONNECTED_BLOCK)?
        .or(store.get_block(LAST_INDEXED_BLOCK)?)
        .or(None);

    if last_connected_block.is_none() {
        info!("No backward sync needed on bootstrap");
        return Ok(());
    }

    let last_connected_block = last_connected_block.unwrap();
    let mut target_number = last_connected_block.number().to_owned();

    // TODO reuse connection
    let rpc_url = "wss://public-node.testnet.rsk.co/websocket";
    let ws = WsConnect::new(rpc_url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    let best_block_num = provider.get_block_number().await? as u64;
    let best_block = fetch_block_data(&provider, None, Some(&best_block_num)).await?; // TODO resilient
    let mut parent_hash: String = best_block.hash().to_string();

    if parent_hash == last_connected_block.hash() {
        info!(
            "No backward sync needed, already at block {} ({})",
            best_block_num,
            best_block.hash()
        );
        return Ok(());
    }

    info!(
        "Running backward sync from block {} ({}) until {} ({})",
        best_block_num,
        parent_hash,
        target_number,
        last_connected_block.hash()
    );

    let mut connection_point_reached = false;

    while !connection_point_reached && !shutdown_flag.is_requested() {
        let recv_block = fetch_block_data(&provider, Some(&parent_hash), None).await?;
        parent_hash = recv_block.parent().to_string();

        debug!(
            "Processing block {} ({}) on backward_sync",
            recv_block.number(),
            recv_block.hash()
        );

        // TODO abstract the key building
        let block_key = &format!("block_{}", recv_block.number());
        let store_block_opt: Option<RskBlock> = store.get_block(block_key)?;

        match store_block_opt {
            None => {
                debug!(
                    "Storing missing block {} ({})",
                    recv_block.number(),
                    recv_block.hash()
                );
                store.save_block(block_key, &recv_block)?;
            }
            Some(store_block) => {
                // TODO pensar en store por hash, simplifica el tema de reorgs

                let same_already_stored = store_block.hash() == recv_block.hash();
                let target_found_and_matching =
                    target_number == store_block.number() && same_already_stored;

                if target_found_and_matching {
                    // we are now fully synced
                    store.save_block(LAST_CONNECTED_BLOCK, &best_block)?;
                    connection_point_reached = true;

                    // TODO remove this tmp useful log
                    let even_parents_matching = store_block.parent() == recv_block.parent();
                    if !even_parents_matching {
                        error!(
                            "Parent mismatch at block {} ({}) and {} ({})",
                            store_block.number(),
                            store_block.hash(),
                            recv_block.number(),
                            recv_block.hash()
                        );
                    }

                    info!(
                        "Finished backward_sync on block {} ({})",
                        recv_block.number(),
                        recv_block.hash()
                    );
                } else if same_already_stored {
                    // we keep moving down in case a previous backward_sync did not reach LAST_CONNECTED_BLOCK due to an interruption
                    debug!(
                        "Already stored block {} ({}), but still trying to reach older connection point",
                        store_block.number(),
                        store_block.hash(),
                    );
                } else {
                    // same number, different hash: reorg - we need to keep moving down until the reorg is fixed
                    target_number = recv_block.number() - 1;

                    // TODO properly cover this behavior with tests, it's hard to replicate on real life

                    warn!(
                        "Reorg found, updating block {} ({}) and target_hash to its parent {}",
                        recv_block.number(),
                        recv_block.hash(),
                        target_number
                    );

                    // TODO abstract the key building
                    let block_key = &format!("block_{}", recv_block.number());
                    store.save_block(block_key, &recv_block)?;
                }
            }
        }
    }

    // TODO try to reuse and therefore close in a unified place
    // TODO check why sometimes I get "WS connection error: Closed" (probably still being reused by some task)
    drop(provider);

    Ok(())
}
