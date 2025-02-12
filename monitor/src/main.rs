use anyhow::Result;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::CachedBlockStore;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use dotenv;
use log::info;
use log_indexer::event_processor::managed_contracts;
use log_indexer::indexer::LogIndexer;
use log_indexer::store::RawLogStore;
use rsk_provider::rpc::AlloyProvider;
use std::env;

fn main() -> Result<()> {
    dotenv::dotenv().expect("Failed to load .env file");
    dotenv::from_filename("block-indexer/.env").expect("Failed to load block-indexer/.env file");

    log4rs::init_file("../log4rs.yml", Default::default()).expect("Failed to load log4rs config");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let block_cache_size = env::var("BLOCK_CACHE_SIZE")
        .expect("BLOCK_CACHE_SIZE not set in env")
        .parse::<usize>()
        .expect("BLOCK_CACHE_SIZE in env must be a number");

    let shutdown_flag = ShutdownFlag::init();

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider = AlloyProvider::new(&rsk_url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    // TODO(iago) try to add prefix on logs for each indexer

    let block_store = CachedBlockStore::new(&format!("{}/blocks", store_path), block_cache_size)?;
    let block_indexer = BlockIndexer::new(
        block_store,
        alloy_provider.clone(),
        &initial_block_hash,
        shutdown_flag.clone(),
    );

    let managed_contracts =
        managed_contracts::load_managed_contracts_from_config("../log-indexer/config")
            .expect("Failed to load managed contracts");

    let log_store = RawLogStore::new(&format!("{}/logs", store_path))?;
    let log_indexer = LogIndexer::new(
        log_store,
        alloy_provider,
        &initial_block_hash,
        managed_contracts,
        shutdown_flag,
    );

    let log_indexer_thread = std::thread::spawn(move || {
        // TODO(iago) properly handle errors
        log_indexer.run().expect("Log indexer failed");
    });

    block_indexer.run()?;

    log_indexer_thread
        .join()
        .expect("Log indexer thread failed");

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
