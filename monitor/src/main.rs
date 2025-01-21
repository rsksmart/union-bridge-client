use anyhow::{anyhow, Result};
use dotenv::dotenv;
use log::warn;
use monitor::indexer::Indexer;
use monitor::rsk_provider::alloy::AlloyProvider;
use monitor::store::CachedBlockStore;
use monitor::utils::ShutdownFlag;
use std::{env, thread};

fn main() -> Result<()> {
    env_logger::init();

    dotenv().expect("Failed to load .env file");

    let shutdown_flag_control = ShutdownFlag::init();
    let shutdown_flag_indexer = shutdown_flag_control.clone();

    ctrlc::set_handler(move || {
        warn!("Ctrl+C received! Signaling worker to stop...");
        shutdown_flag_control.set_on();
    })
    .expect("Error setting Ctrl+C handler");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let store = CachedBlockStore::new(&store_path)
        .expect("Failed to create CachedKeyValueStore (unrecoverable)");

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider =
        AlloyProvider::new(&rsk_url).expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");
    let indexer = Indexer::new(store, alloy_provider, &initial_block_hash);

    run_indexer(indexer, shutdown_flag_indexer)?;

    log::logger().flush();

    Ok(())
}

fn run_indexer(
    indexer: Indexer<AlloyProvider, CachedBlockStore>,
    shutdown_flag: ShutdownFlag,
) -> Result<()> {
    let worker_thread = thread::spawn(move || indexer.run(shutdown_flag));

    worker_thread.join().map_err(|e| {
        anyhow!(
            "The worker_thread has errored with message: {:?}",
            e.downcast_ref::<String>()
                .unwrap_or(&"Unknown error".to_string())
        )
    })??;

    Ok(())
}
