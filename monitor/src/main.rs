use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::PubSubFrontend;
use anyhow::{anyhow, bail, Ok, Result};
use log::{debug, error, info, warn};
use monitor::provider::{
    AlloyBlockSubscription, AlloyLogsSubscription, AlloyRskWsProvider, RskSubscription,
    RskWsProvider,
};
use monitor::store::CachedKeyValueStore;
use monitor::types::{RskBlock, RskRpcBlock};
use monitor::utils::RuntimeSync;
use serde_json::{json, Value};
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, thread, time::Duration};

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK_HASH_ENV: &str =
    "0x5609fff226ca052d12eca7bfdb45edca1c8252ac08b492420990fc8fb82c2868";

fn main() -> Result<()> {
    env_logger::init();

    let envs = dotenv::dotenv();

    if envs.is_err() {
        warn!("No .env file found");
    }

    // TODO(iago) move its creation to the Provider file
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
            error!("Unrecoverable error in backward_sync: {:?}", e);
            return;
        }

        if shutdown_flag_worker.is_on() {
            return;
        }

        if let Err(e) = subscribe_blocks(&shutdown_flag_worker, &store, rt_sync.clone()) {
            error!("Unrecoverable error in subscribe_blocks: {:?}", e);
            return;
        }
    });

    worker_thread.join().unwrap_or_else(|e| {
        if let Some(err) = e.downcast_ref::<&str>() {
            error!("The worker_thread has errored with message: {}", err);
            return;
        } else if let Some(err) = e.downcast_ref::<String>() {
            error!("The worker_thread has errored with message: {}", err);
            return;
        } else {
            error!("The worker_thread has errored with an unknown type");
            return;
        }
    });

    info!("Worker threads completed, shutting down now.");

    log::logger().flush();

    Ok(())
}

fn subscribe_blocks(
    shutdown_flag: &ShutdownFlag,
    store: &CachedKeyValueStore,
    rt_sync: Arc<RuntimeSync>,
) -> Result<()> {
    // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15

    let rsk_ws_provider: Box<
        dyn RskWsProvider<BlockSub = AlloyBlockSubscription, LogsSub = AlloyLogsSubscription>,
    > = Box::new(AlloyRskWsProvider::new(
        "wss://public-node.testnet.rsk.co/websocket",
        rt_sync.clone(),
    )?);

    let mut rsk_block_subscription = rsk_ws_provider.subscribe_blocks()?;

    info!("Start subscribe_blocks...");

    let mut parent_block_hash = store
        .get_best_block()?
        .map(|b| b.hash().to_string())
        .ok_or_else(|| anyhow!("Failed to get best_block from store"))?;

    while !shutdown_flag.is_on() {
        let new_block_hash = rsk_block_subscription.next()?;
        if new_block_hash.is_none() {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let recv_block = rsk_ws_provider.get_block_by_hash(&new_block_hash.unwrap())?;
        info!(
            "Processing block {} ({}) on subscription",
            recv_block.number(),
            recv_block.hash()
        );

        debug!("Fetched RSK block on subscription: {:?}", recv_block);

        if recv_block.parent() == parent_block_hash {
            // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
            store.save_block(&recv_block)?;
            store.set_canonical_block(&recv_block)?;
            store.set_best_block(&recv_block)?;
            store.set_last_connected_block(&recv_block)?;
        } else {
            warn!(
                "Reorg or gap detected on block {} ({} -> {})",
                recv_block.number(),
                parent_block_hash,
                recv_block.parent()
            );
            backward_sync(&shutdown_flag, rt_sync.clone(), &store)?;
        }

        parent_block_hash = recv_block.hash().to_string();
    }

    // TODO(iago) do this close outside if reused by http calls

    rsk_block_subscription.unsubscribe()?;

    Ok(())
}

// fn apply_reorg(store: &CachedKeyValueStore, hash_for_reorg: &str) -> Result<()> {
//     let mut next_canonical_hash = hash_for_reorg.to_string();
//
//     loop {
//         let new_block = store
//             .get_block_by_hash(&next_canonical_hash)?
//             .ok_or_else(|| anyhow!("Could not find new_block by hash {}", next_canonical_hash))?;
//
//         let old_block = store
//             .get_block_by_number(new_block.number())?
//             .ok_or_else(|| anyhow!("Could not find old_block by number {}", new_block.number()))?;
//
//         if new_block.hash() == old_block.hash() {
//             info!(
//                 "Reorg fixed on block {} ({})",
//                 new_block.number(),
//                 new_block.hash()
//             );
//             break;
//         }
//
//         info!(
//             "Fixing reorg on block {} ({} -> {})",
//             new_block.number(),
//             old_block.hash(),
//             new_block.hash()
//         );
//
//         store.set_canonical_block(&new_block)?;
//
//         next_canonical_hash = new_block.parent().to_string();
//     }
//
//     Ok(())
// }

// TODO(iago) move to provider
fn fetch_block_data(
    provider: &RootProvider<PubSubFrontend>,
    rt_sync: Arc<RuntimeSync>,
    block_hash: Option<&str>,
    block_number: Option<&u64>,
) -> Result<RskBlock> {
    if block_hash.is_none() && block_number.is_none() {
        bail!("Either block_hash or block_number must be provided");
    }

    if block_hash.is_some() && block_number.is_some() {
        bail!("Only one of block_hash or block_number must be provided");
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

    // TODO(iago) resilience when response is not a block (ie not found)

    let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
    let rsk_block: RskBlock = RskBlock::from(rpc_block);

    Ok(rsk_block)
}

fn backward_sync(
    shutdown_flag: &ShutdownFlag,
    rt_sync: Arc<RuntimeSync>,
    store: &CachedKeyValueStore,
) -> Result<()> {
    // TODO(iago) reuse connection
    // TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
    let rpc_url = "wss://public-node.testnet.rsk.co/websocket";
    let ws = WsConnect::new(rpc_url);
    let provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;

    initialize_db_if_required(&rt_sync, store, &provider)?;

    let last_connected_block = store
        .get_last_connected_block()?
        .ok_or_else(|| anyhow!("Failed to get last_connected_block from store"))?;

    let best_block_num = rt_sync.run(provider.get_block_number())?;
    let best_block = fetch_block_data(&provider, rt_sync.clone(), None, Some(&best_block_num))?; // TODO(iago) resilient to not found

    if best_block.hash() == last_connected_block.hash() {
        info!(
            "No backward sync needed, already at block {} ({})",
            best_block_num,
            best_block.hash()
        );
        return Ok(());
    }

    info!(
        "Running backward_sync from block {} ({}) to {} ({})",
        best_block_num,
        best_block.hash(),
        last_connected_block.number(),
        last_connected_block.hash(),
    );

    store.set_best_block(&best_block)?;

    let mut target_block_num = last_connected_block.number();
    let mut connection_point_reached = false;

    let mut node_block: RskBlock = best_block.clone();
    let mut store_block_opt = store.get_block_by_number(node_block.number())?;

    while !shutdown_flag.is_on() && !connection_point_reached {
        let is_reorg = store_block_opt
            .as_ref()
            .map(|sb| sb.hash() != node_block.hash())
            .unwrap_or(false);

        if store_block_opt.is_none() || is_reorg {
            info!(
                "Creating or updating block {} ({})...",
                node_block.number(),
                node_block.hash()
            );

            // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
            store.save_block(&node_block)?;
            store.set_canonical_block(&node_block)?;
        } else {
            debug!(
                "Already stored block {} ({}) while trying to reach connection_hash, nothing to do",
                node_block.number(),
                node_block.hash()
            );
        }

        // TODO(Jira) request and persist uncles: https://rsklabs.atlassian.net/browse/UB-16

        let parent_block_num = node_block.number() - 1;
        node_block = fetch_block_data(&provider, rt_sync.clone(), None, Some(&parent_block_num))?;
        store_block_opt = store.get_block_by_number(parent_block_num)?;

        let is_target_num_reached = store_block_opt
            .as_ref()
            .map(|sb| sb.number() <= target_block_num)
            .unwrap_or(false);

        if is_target_num_reached {
            let store_block = store_block_opt.as_ref().unwrap();
            if store_block.hash() == node_block.hash() {
                connection_point_reached = true;
                store.set_last_connected_block(&best_block)?;
                debug!("backward_sync completed!");
            } else {
                debug!("target_block_num changed to {}", store_block.number(),);
                target_block_num -= 1;
            }
        }
    }

    // TODO(iago) try to reuse and therefore close in a unified place
    drop(provider);

    Ok(())
}

fn initialize_db_if_required(
    rt_sync: &Arc<RuntimeSync>,
    store: &CachedKeyValueStore,
    provider: &RootProvider<PubSubFrontend>,
) -> Result<()> {
    let initial_block: Option<RskBlock> = store.get_block_by_hash(INITIAL_BLOCK_HASH_ENV)?.or(None);

    match initial_block {
        Some(_) => Ok(()),
        None => {
            let initial_block = fetch_block_data(
                &provider,
                rt_sync.clone(),
                Some(INITIAL_BLOCK_HASH_ENV),
                None,
            )?; // TODO(iago) resilient to not found

            info!(
                "First backward sync, setting last_connected_block to initial block {} ({})",
                initial_block.number(),
                initial_block.hash()
            );

            // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
            // initialize the store with the initial block info
            store.set_canonical_block(&initial_block)?;
            store.set_best_block(&initial_block)?;
            store.set_last_connected_block(&initial_block)?;
            // should go last, as it will be used to determine if the DB is initialized
            store.save_block(&initial_block)?;

            Ok(())
        }
    }
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
