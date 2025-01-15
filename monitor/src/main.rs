use anyhow::{anyhow, Ok, Result};
use log::{debug, error, info, warn};
use monitor::provider::{
    AlloyProvider, RskApi, RskBlockSubscription, RskBlockSubscriptionApi, RskProvider,
};
use monitor::store::CachedKeyValueStore;
use monitor::types::RskBlock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time::Duration};

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const WS_URL: &'static str = "wss://public-node.testnet.rsk.co/websocket";

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK_HASH_ENV: &str =
    "0xd608130f2caf657d11ec5bc2cbe7c17415813cb906714d1f2b4c6079dcf4c39a";

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14

fn main() -> Result<()> {
    env_logger::init();

    let envs = dotenv::dotenv();

    if envs.is_err() {
        warn!("No .env file found");
    }

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
        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        let rsk_provider = RskApi::new(WS_URL);

        // TODO(iago) extract monitor to its own class
        if let Err(e) = run_monitor(&shutdown_flag_worker, &store, &rsk_provider) {
            error!("Unrecoverable error running monitor: {:?}", e);
        }

        if let Err(e) = rsk_provider.disconnect() {
            error!("Failed to close provider: {:?}", e);
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

fn run_monitor(
    shutdown_flag: &Arc<ShutdownFlag>,
    store: &CachedKeyValueStore,
    rsk_provider: &RskApi<AlloyProvider>,
) -> Result<()> {
    // After boot, we do a backward_sync to catch up with the latest block
    backward_sync(&shutdown_flag, &rsk_provider, &store)?;

    if shutdown_flag.is_on() {
        return Ok(());
    }

    subscribe_blocks(&shutdown_flag, &store, &rsk_provider)?;

    Ok(())
}

fn subscribe_blocks(
    shutdown_flag: &ShutdownFlag,
    store: &CachedKeyValueStore,
    provider: &RskApi<AlloyProvider>,
) -> Result<()> {
    info!("Start subscribe_blocks...");

    let mut rsk_block_subscription = RskBlockSubscriptionApi::new(&provider);

    let mut tip_block = store
        .get_best_block()?
        .ok_or_else(|| anyhow!("Failed to get best_block from store"))?;

    while !shutdown_flag.is_on() {
        let new_block = rsk_block_subscription.try_next()?;
        if new_block.is_none() {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let new_block = new_block.unwrap();

        debug!("Fetched RSK block on subscription: {:?}", new_block);

        // TODO(iago) pensar cuando hay que poner como best_block, si con el hash enganchado sirve o debería comprobar num consecutivo tb
        // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11

        // we always save the block by hash
        store.save_block(&new_block)?;

        let extends_canonical = new_block.parent() == tip_block.hash();
        let requires_local_reorg =
            !extends_canonical && new_block.total_difficulty() > tip_block.total_difficulty();

        if extends_canonical {
            // set canonical fields
            store.set_best_block(&new_block)?;
            store.set_canonical_block(&new_block)?;
            // set last connected block to this new best block
            store.set_last_connected_block(&new_block)?;

            info!(
                "Processed block {} ({}): new tip",
                new_block.number(),
                new_block.hash()
            );

            tip_block = new_block;
        } else if requires_local_reorg {
            info!(
                "Processed block {} ({}): local reorg, run backward sync",
                new_block.number(),
                new_block.hash()
            );
            // backward_sync fixes reorgs internally (if any)
            tip_block = backward_sync(&shutdown_flag, &provider, &store)?;
        } else {
            info!(
                "Processing block {} ({}): non extending, non competing",
                new_block.number(),
                new_block.hash()
            );
        }
    }

    // TODO(iago) ensure closed even on errors above
    rsk_block_subscription.unsubscribe()?;

    Ok(())
}

fn backward_sync(
    shutdown_flag: &ShutdownFlag,
    rsk_provider: &RskApi<AlloyProvider>,
    store: &CachedKeyValueStore,
) -> Result<RskBlock> {
    initialize_db_if_required(store, rsk_provider)?;

    let last_connected_block = store
        .get_last_connected_block()?
        .ok_or_else(|| anyhow!("Failed to get last_connected_block from store"))?;

    let best_block = rsk_provider.get_best_block()?; // TODO(iago) resilient to not found

    if best_block.hash() == last_connected_block.hash() {
        info!(
            "No backward sync needed, already at block {} ({})",
            best_block.number(),
            best_block.hash()
        );
        return Ok(best_block);
    }

    info!(
        "Running backward_sync from block {} ({}) to {} ({})",
        best_block.number(),
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
        node_block = rsk_provider.get_block_by_number(parent_block_num)?;
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
                info!("backward_sync completed!");
            } else {
                target_block_num -= 1;
                debug!("decrementing target block: {}", target_block_num);
            }
        }
    }

    Ok(best_block)
}

fn initialize_db_if_required(
    store: &CachedKeyValueStore,
    provider: &RskApi<AlloyProvider>,
) -> Result<()> {
    let initial_block: Option<RskBlock> = store.get_block_by_hash(INITIAL_BLOCK_HASH_ENV)?.or(None);

    match initial_block {
        Some(_) => Ok(()),
        None => {
            let initial_block = provider.get_block_by_hash(INITIAL_BLOCK_HASH_ENV)?; // TODO(iago) resilient to not found

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
