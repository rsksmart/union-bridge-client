use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, SubscriptionItem};
use anyhow::{anyhow, bail, Ok, Result};
use log::{debug, error, info, trace, warn};
use monitor::store::CachedKeyValueStore;
use monitor::types::{RskBlock, RskRpcBlock};
use serde_json::{json, Value};
use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, sync::mpsc::channel, thread, time::Duration};
use tokio::runtime::Runtime;

// TODO use keys enum
const LAST_INDEXED_BLOCK: &'static str = "LAST_INDEXED_BLOCK";
const LAST_CONNECTED_BLOCK: &'static str = "LAST_CONNECTED_BLOCK";

// TODO convert to sync code, apparently it is safer
fn main() -> Result<()> {
    env_logger::init();

    let rt_sync = RuntimeSync::new()?;

    let shutdown_flag_control = Arc::new(ShutdownFlag::init());
    let shutdown_flag_worker = Arc::clone(&shutdown_flag_control);

    ctrlc::set_handler(move || {
        println!("Ctrl+C received! Signaling worker to stop...");
        shutdown_flag_control.set_on();
    })
    .expect("Error setting Ctrl+C handler");

    let store = CachedKeyValueStore::new("/Users/illuque/tmp/")
        .expect("Failed to create CachedKeyValueStore");

    let worker_thread = thread::spawn(move || {
        // after boot, we do a backward_sync to catch up with the latest block
        if let Err(e) = backward_sync(&shutdown_flag_worker, &rt_sync, &store) {
            error!("Unrecoverable error in test_backward: {:?}", e);
        }

        if let Err(e) = test_subscribe(&shutdown_flag_worker) {
            error!("Unrecoverable error in test_subscribe: {:?}", e);
        }
    });

    worker_thread.join().unwrap_or_else(|e| {
        if let Some(err) = e.downcast_ref::<&str>() {
            error!("The worker_thread has errored with message: {}", err);
        } else if let Some(err) = e.downcast_ref::<String>() {
            error!("The worker_thread has errored with message: {}", err);
        } else {
            error!("The worker_thread has errored with an unknown type");
        }
    });

    log::logger().flush();

    Ok(())
}

fn test_subscribe(shutdown_flag: &Arc<ShutdownFlag>) -> Result<()> {
    while !shutdown_flag.is_on() {
        info!("Waiting for shutdown signal on subscribe...");
        thread::sleep(Duration::from_secs(5));
    }
    Ok(())
}

fn fetch_block_data(
    provider: &RootProvider<PubSubFrontend>,
    rt_sync: &RuntimeSync,
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

    let response: Value = rt_sync.run(
        provider
            .client()
            .request(method, vec![json!(block_id), json!(false)]),
    )?;

    // TODO resilience when response is not a block (ie not found)

    let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
    let rsk_block: RskBlock = RskBlock::from(rpc_block);

    Ok(rsk_block)
}

fn backward_sync(
    shutdown_flag: &ShutdownFlag,
    rt_sync: &RuntimeSync,
    store: &CachedKeyValueStore,
) -> Result<()> {
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

    let provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;
    let best_block_num = rt_sync.run(provider.get_block_number())?;

    let best_block = fetch_block_data(&provider, rt_sync, None, Some(&best_block_num))?; // TODO resilient
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

    while !connection_point_reached && !shutdown_flag.is_on() {
        let recv_block = fetch_block_data(&provider, rt_sync, Some(&parent_hash), None)?;
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

    pub fn set_on(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_on(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

struct RuntimeSync {
    rt: Runtime,
}

impl RuntimeSync {
    pub fn new() -> Result<Self> {
        let rt = Runtime::new()?;
        Ok(RuntimeSync { rt })
    }

    pub fn run<Fut, RetType, Err>(&self, future: Fut) -> Result<RetType>
    where
        Fut: Future<Output = Result<RetType, Err>>,
        RetType: Send + 'static,
        Err: Error,
    {
        self.rt.block_on(async {
            future
                .await
                .map_err(|e| anyhow!("Error in run_sync: {:?}", e))
        })
    }
}
