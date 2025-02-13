use anyhow::Result;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use dotenv::dotenv;
use log::info;
use log_indexer::managed_contracts;
use log_indexer::indexer::LogIndexer;
use log_indexer::store::RawLogStore;
use rsk_provider::rpc::AlloyProvider;
use std::env;

fn main() -> Result<()> {
    dotenv().expect("Failed to load .env file");
    dotenv::from_filename("../.env").expect("Failed to load global .env file");

    log4rs::init_file("../log4rs.yml", Default::default()).expect("Failed to load log4rs config");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let store = RawLogStore::new(&format!("{}/logs", store_path))?;

    let shutdown_flag = ShutdownFlag::init();

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider = AlloyProvider::new(&rsk_url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    let managed_contracts = managed_contracts::load_managed_contracts_from_config("./config")
        .expect("Failed to load managed contracts");

    let indexer = LogIndexer::new(
        store,
        alloy_provider,
        &initial_block_hash,
        managed_contracts,
        shutdown_flag,
    );

    indexer.run()?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
