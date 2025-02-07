use anyhow::Result;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::CachedBlockStore;
use common::rsk_indexer::RskIndexer;
use dotenv::dotenv;
use log::info;
use log_indexer::indexer::LogIndexer;
use log_indexer::store::RawLogStore;
use rsk_provider::alloy::AlloyProvider;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use std::env;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() -> Result<()> {
    dotenv().expect("Failed to load .env file");

    log4rs::init_file("../log4rs.yml", Default::default()).expect("Failed to load log4rs config");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider =
        AlloyProvider::new(&rsk_url).expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    flag::register(SIGINT, Arc::clone(&shutdown_flag)).expect("Failed to set SIGINT handler");
    flag::register(SIGTERM, Arc::clone(&shutdown_flag)).expect("Failed to set SIGTERM handler");

    // TODO(iago) try to add prefix on logs for each indexer

    let block_store = CachedBlockStore::new(&format!("{}/blocks", store_path))?;
    let block_indexer = BlockIndexer::new(
        block_store,
        alloy_provider.clone(), // TODO(iago) try to use Rc instead of cloning
        &initial_block_hash,
        shutdown_flag.clone(),
    );

    let log_store = RawLogStore::new(&format!("{}/logs", store_path))?;
    let log_indexer = LogIndexer::new(
        log_store,
        alloy_provider,
        &initial_block_hash,
        shutdown_flag.clone(),
    );

    let log_indexer_thread = std::thread::spawn(move || {
        // TODO(iago) properly handle errors
        log_indexer.run().expect("Log indexer failed");
    });

    block_indexer.run()?;

    // TODO(iago) properly handle errors
    log_indexer_thread
        .join()
        .expect("Log indexer thread failed");

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
