use anyhow::Result;
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::CachedBlockStore;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use dotenv::dotenv;
use log::info;
use rsk_provider::rpc::AlloyProvider;
use std::env;

fn main() -> Result<()> {
    dotenv().expect("Failed to load .env file");

    log4rs::init_file("../log4rs.yml", Default::default()).expect("Failed to load log4rs config");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let block_cache_size = env::var("BLOCK_CACHE_SIZE")
        .expect("BLOCK_CACHE_SIZE not set in env")
        .parse::<usize>()
        .expect("BLOCK_CACHE_SIZE in env must be a number");
    let store = CachedBlockStore::new(&format!("{}/blocks", store_path), block_cache_size)?;

    let shutdown_flag = ShutdownFlag::init();

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider = AlloyProvider::new(&rsk_url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    let indexer = BlockIndexer::new(store, alloy_provider, &initial_block_hash, shutdown_flag);

    indexer.run()?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
