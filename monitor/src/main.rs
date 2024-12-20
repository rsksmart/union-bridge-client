use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, SubscriptionItem};
use anyhow::{anyhow, bail, Ok, Result};
use log::{debug, error, info, trace, warn};
use monitor::provider::{
    AlloyBlockSubscription, AlloyLogsSubscription, AlloyRskWsProvider, RskSubscription,
    RskWsProvider,
};
use monitor::store::CachedKeyValueStore;
use monitor::types::{RskBlock, RskLog, RskRpcBlock};
use monitor::utils::RuntimeSync;
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
        // if let Err(e) = backward_sync(&shutdown_flag_worker, &rt_sync, &store) {
        //     error!("Unrecoverable error in test_backward: {:?}", e);
        // }

        if let Err(e) = subscribe_blocks_loop(&shutdown_flag_worker, &store) {
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
    )?);

    let mut rsk_block_subscription = rsk_ws_provider.subscribe_blocks()?;

    info!("Subscribed to new block headers");

    // some last second gap backward_sync in case while this task was starting we missed some block
    // TODO resume this run
    //backward_sync(shutdown_flag, store).await?;

    info!("Start receiving blocks from eth_subscribe...");

    let mut last_received_block_opt: Option<RskBlock> = None;

    while !shutdown_flag.is_on() {
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

        // TODO resume this run
        // check_reorg(&last_received_block_opt, &new_block_hash);

        // TODO move to store abstraction class
        // TODO create keys enum
        let block_key = &format!("block_{}", new_block.number());
        store.save_block(block_key, &new_block)?;
        // TODO create keys enum
        store.save_block(LAST_INDEXED_BLOCK, &new_block)?;
        last_received_block_opt = Some(new_block);
    }

    // TODO check this, I think it is not needed, that it is wrong actually
    // if let Some(last_received_block) = last_received_block_opt {
    //     // update connection point to best block
    //     store.save_block(LAST_CONNECTED_BLOCK, &last_received_block)?;
    // }

    info!("Shutdown requested, stopping subscribe_blocks...");

    // TODO do this close outside if reused by http calls
    // TODO check why sometimes I get "WS connection error: Closed" (probably still being reused by some task)
    // TODO run eth_unsubscribe with subscription id
    // TODO handle reconnect on error

    rsk_block_subscription.unsubscribe()?;

    Ok(())
}

// fn backward_sync(
//     shutdown_flag: &ShutdownFlag,
//     rt_sync: &RuntimeSync,
//     store: &CachedKeyValueStore,
// ) -> Result<()> {
//     // TODO improve this logic passing last_connected_block as parameter
//     // we have to reach LAST_CONNECTED_BLOCK:
//     // - if LAST_CONNECTED_BLOCK not present (first time app is run) we have to reach LAST_INDEXED_BLOCK
//     // - if LAST_INDEXED_BLOCK not present either (first time app is run), we skip the sync
//     let last_connected_block: Option<RskBlock> = store
//         .get_block(LAST_CONNECTED_BLOCK)?
//         .or(store.get_block(LAST_INDEXED_BLOCK)?)
//         .or(None);
//
//     if last_connected_block.is_none() {
//         info!("No backward sync needed on bootstrap");
//         return Ok(());
//     }
//
//     let last_connected_block = last_connected_block.unwrap();
//     let mut target_number = last_connected_block.number().to_owned();
//
//     // TODO reuse connection
//     let rpc_url = "wss://public-node.testnet.rsk.co/websocket";
//     let ws = WsConnect::new(rpc_url);
//
//     let provider = rt_sync.run(ProviderBuilder::new().on_ws(ws))?;
//     let best_block_num = rt_sync.run(provider.get_block_number())?;
//
//     let best_block = fetch_block_data(&provider, rt_sync, None, Some(&best_block_num))?; // TODO resilient
//     let mut parent_hash: String = best_block.hash().to_string();
//
//     if parent_hash == last_connected_block.hash() {
//         info!(
//             "No backward sync needed, already at block {} ({})",
//             best_block_num,
//             best_block.hash()
//         );
//         return Ok(());
//     }
//
//     info!(
//         "Running backward sync from block {} ({}) until {} ({})",
//         best_block_num,
//         parent_hash,
//         target_number,
//         last_connected_block.hash()
//     );
//
//     let mut connection_point_reached = false;
//
//     while !connection_point_reached && !shutdown_flag.is_on() {
//         let recv_block = fetch_block_data(&provider, rt_sync, Some(&parent_hash), None)?;
//         parent_hash = recv_block.parent().to_string();
//
//         debug!(
//             "Processing block {} ({}) on backward_sync",
//             recv_block.number(),
//             recv_block.hash()
//         );
//
//         // TODO abstract the key building
//         let block_key = &format!("block_{}", recv_block.number());
//         let store_block_opt: Option<RskBlock> = store.get_block(block_key)?;
//
//         match store_block_opt {
//             None => {
//                 debug!(
//                     "Storing missing block {} ({})",
//                     recv_block.number(),
//                     recv_block.hash()
//                 );
//                 store.save_block(block_key, &recv_block)?;
//             }
//             Some(store_block) => {
//                 // TODO pensar en store por hash, simplifica el tema de reorgs
//
//                 let same_already_stored = store_block.hash() == recv_block.hash();
//                 let target_found_and_matching =
//                     target_number == store_block.number() && same_already_stored;
//
//                 if target_found_and_matching {
//                     // we are now fully synced
//                     store.save_block(LAST_CONNECTED_BLOCK, &best_block)?;
//                     connection_point_reached = true;
//
//                     // TODO remove this tmp useful log
//                     let even_parents_matching = store_block.parent() == recv_block.parent();
//                     if !even_parents_matching {
//                         error!(
//                             "Parent mismatch at block {} ({}) and {} ({})",
//                             store_block.number(),
//                             store_block.hash(),
//                             recv_block.number(),
//                             recv_block.hash()
//                         );
//                     }
//
//                     info!(
//                         "Finished backward_sync on block {} ({})",
//                         recv_block.number(),
//                         recv_block.hash()
//                     );
//                 } else if same_already_stored {
//                     // we keep moving down in case a previous backward_sync did not reach LAST_CONNECTED_BLOCK due to an interruption
//                     debug!(
//                         "Already stored block {} ({}), but still trying to reach older connection point",
//                         store_block.number(),
//                         store_block.hash(),
//                     );
//                 } else {
//                     // same number, different hash: reorg - we need to keep moving down until the reorg is fixed
//                     target_number = recv_block.number() - 1;
//
//                     // TODO properly cover this behavior with tests, it's hard to replicate on real life
//
//                     warn!(
//                         "Reorg found, updating block {} ({}) and target_hash to its parent {}",
//                         recv_block.number(),
//                         recv_block.hash(),
//                         target_number
//                     );
//
//                     // TODO abstract the key building
//                     let block_key = &format!("block_{}", recv_block.number());
//                     store.save_block(block_key, &recv_block)?;
//                 }
//             }
//         }
//     }
//
//     // TODO try to reuse and therefore close in a unified place
//     // TODO check why sometimes I get "WS connection error: Closed" (probably still being reused by some task)
//     drop(provider);
//
//     Ok(())
// }

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
