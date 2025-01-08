use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::PubSubFrontend;
use anyhow::{bail, Ok, Result};
use log::{debug, error, info, warn};
use monitor::provider::{
    AlloyBlockSubscription, AlloyLogsSubscription, AlloyRskWsProvider, RskSubscription,
    RskWsProvider,
};
use monitor::store::CachedKeyValueStore;
use monitor::types::{RskBlock, RskRpcBlock};
use monitor::utils::RuntimeSync;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, thread, time::Duration};

// TODO use keys enum
const BLOCK_TO_CONNECT: &'static str = "BLOCK_TO_CONNECT";
const INITIAL_BLOCK_ENV: &'static str = "INITIAL_BLOCK";

// TODO convert to sync code, apparently it is safer
fn main() -> Result<()> {
    env_logger::init();

    let envs = dotenv::dotenv();

    if envs.is_err() {
        warn!("No .env file found");
    }

    let rt_sync = Arc::new(RuntimeSync::new()?);

    let shutdown_flag_control = Arc::new(ShutdownFlag::init());
    let shutdown_flag_worker = Arc::clone(&shutdown_flag_control);

    ctrlc::set_handler(move || {
        warn!("Ctrl+C received! Signaling worker to stop...");
        shutdown_flag_control.set_on();
    })
    .expect("Error setting Ctrl+C handler");

    let store = CachedKeyValueStore::new("/Users/illuque/tmp/")
        .expect("Failed to create CachedKeyValueStore");

    let worker_thread = thread::spawn(move || {
        // after boot, we do a backward_sync to catch up with the latest block
        if let Err(e) = backward_sync(&shutdown_flag_worker, rt_sync.clone(), &store) {
            error!("Unrecoverable error in test_backward: {:?}", e);
        }

        if let Err(e) = subscribe_blocks_loop(&shutdown_flag_worker, &store, rt_sync.clone()) {
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

fn subscribe_blocks_loop(
    shutdown_flag: &ShutdownFlag, // TODO instead of receiving this, try to receive the ws subscription and close it on ctrl+c, that should stop the loop also
    store: &CachedKeyValueStore,
    rt_sync: Arc<RuntimeSync>,
) -> Result<()> {
    // TODO (make resilient, see list below)
    // 1) add last_ping to check if connection is still alive
    // 2) add exponential backoff
    // 3) add reconnect
    // 4) add backup servers
    // 5) everything configurable

    let rsk_ws_provider: Box<
        dyn RskWsProvider<BlockSub = AlloyBlockSubscription, LogsSub = AlloyLogsSubscription>,
    > = Box::new(AlloyRskWsProvider::new(
        "wss://public-node.testnet.rsk.co/websocket",
        rt_sync.clone(),
    )?);

    let mut rsk_block_subscription = rsk_ws_provider.subscribe_blocks()?;

    info!("Subscribed to new block headers");

    // some last second gap backward_sync in case while this task was starting we missed some block
    if let Err(e) = backward_sync(&shutdown_flag, rt_sync.clone(), &store) {
        error!("Unrecoverable error in test_backward: {:?}", e);
    }

    info!("Start receiving blocks from eth_subscribe...");

    loop {
        let new_block_hash = rsk_block_subscription.next()?;
        if new_block_hash.is_none() {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let new_block = rsk_ws_provider.get_block_by_hash(&new_block_hash.unwrap())?;
        debug!("Fetched Rsk block on subscription: {:?}", new_block);
        info!(
            "Processing block {} ({}) on subscription",
            new_block.number(),
            new_block.hash()
        );

        // TODO move to store abstraction class
        // TODO create keys enum
        let block_key = &format!("block_{}", new_block.number());
        store.save_block(block_key, &new_block)?;

        if shutdown_flag.is_on() {
            info!("Shutdown requested, stopping subscribe_blocks and changing BLOCK_TO_CONNECT to {} ({})...", new_block.number(), new_block.hash());
            store.save_block(BLOCK_TO_CONNECT, &new_block)?;
            break;
        }
    }

    // TODO do this close outside if reused by http calls
    // TODO check why sometimes I get "WS connection error: Closed" (probably still being reused by some task)
    // TODO run eth_unsubscribe with subscription id
    // TODO handle reconnect on error

    rsk_block_subscription.unsubscribe()?;

    Ok(())
}

// TODO remove
fn fetch_block_data(
    provider: &RootProvider<PubSubFrontend>,
    rt_sync: Arc<RuntimeSync>,
    block_hash: Option<&str>,
    block_number: Option<&u64>,
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

    let rpc_call = provider
        .client()
        .request(method, vec![json!(block_id), json!(false)]);

    let response = rt_sync.run(rpc_call)?;

    // TODO resilience when response is not a block (ie not found)

    let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
    let rsk_block: RskBlock = RskBlock::from(rpc_block);

    Ok(rsk_block)
}

fn backward_sync(
    shutdown_flag: &ShutdownFlag,
    rt_sync: Arc<RuntimeSync>,
    store: &CachedKeyValueStore,
) -> Result<()> {
    // TODO improve this logic passing BLOCK_TO_CONNECT as parameter
    let block_to_connect_db: Option<RskBlock> = store.get_block(BLOCK_TO_CONNECT)?.or(None);

    // TODO reuse connection
    let rpc_url = "wss://public-node.testnet.rsk.co/websocket";
    let ws = WsConnect::new(rpc_url);
    let provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;

    let block_to_connect = match block_to_connect_db {
        Some(block) => block,
        None => {
            let monitor_genesis_hash =
                "0x1e84101802a707d1c9fb135a6557bd3849a0feb4ac8ace6bcf6329dbcbaeeed2"; // TODO get from env
            let monitor_genesis_block =
                fetch_block_data(&provider, rt_sync.clone(), Some(monitor_genesis_hash), None)?; // TODO resilient to not found

            info!(
                "First backward sync, setting BLOCK_TO_CONNECT to genesis from env {} ({})",
                monitor_genesis_block.number(),
                monitor_genesis_block.hash()
            );

            let block_key = &format!("block_{}", monitor_genesis_block.number());
            store.save_block(block_key, &monitor_genesis_block)?;

            monitor_genesis_block
        }
    };

    let best_block_num = rt_sync.run(provider.get_block_number())?;
    let best_block = fetch_block_data(&provider, rt_sync.clone(), None, Some(&best_block_num))?; // TODO resilient to not found

    if best_block.hash() == block_to_connect.hash() {
        info!(
            "No backward sync needed, already at block {} ({})",
            best_block_num,
            best_block.hash()
        );
        return Ok(());
    }

    info!(
        "Running backward_sync from block {} ({}) until {} ({})",
        best_block_num,
        best_block.hash(),
        block_to_connect.number(),
        block_to_connect.hash(),
    );

    let mut target_hash = block_to_connect.hash().to_string();
    let mut parent_hash: String = best_block.hash().to_string();

    while !shutdown_flag.is_on() {
        let recv_block: RskBlock =
            fetch_block_data(&provider, rt_sync.clone(), Some(&parent_hash), None)?;

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
                let is_reorg = store_block.hash() != recv_block.hash();
                if is_reorg {
                    // we need to retarget if reorg detected on block_to_connect
                    if store_block.number() <= block_to_connect.number() {
                        warn!(
                            "Reorg detected on connection point, retargeting to {} ({})",
                            recv_block.number() - 1,
                            recv_block.parent()
                        );
                        target_hash = recv_block.parent().to_string();
                    } else {
                        warn!(
                            "Reorg detected, replacing block {} ({}) by {}",
                            store_block.number(),
                            store_block.hash(),
                            recv_block.hash(),
                        );
                    }

                    // TODO abstract the key building
                    let block_key = &format!("block_{}", recv_block.number());
                    store.save_block(block_key, &recv_block)?;

                    continue;
                }

                let target_found = !is_reorg && store_block.hash() == target_hash;
                if target_found {
                    store.save_block(BLOCK_TO_CONNECT, &best_block)?;

                    info!(
                        "Finished backward_sync on block {} ({})",
                        recv_block.number(),
                        recv_block.hash()
                    );

                    break;
                }

                // this may happen when the backward_sync is stopped before finishing
                debug!(
                    "Skipping already stored block {} ({}) while trying to reach connection point",
                    recv_block.number(),
                    recv_block.hash()
                );
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
