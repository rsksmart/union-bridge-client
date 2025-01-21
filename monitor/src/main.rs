use anyhow::{anyhow, Result};
use log::warn;
use monitor::indexer::Indexer;
use monitor::rsk_provider::alloy::AlloyProvider;
use monitor::store::CachedBlockStore;
use monitor::utils::ShutdownFlag;
use std::thread;

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const WS_URL: &str = "wss://public-node.testnet.rsk.co/websocket";

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK_HASH_ENV: &str =
    "0x551c09b6d4e35008a83016a16922676059eab39ba1c72d2c634c1c9119158a4a";

fn main() -> Result<()> {
    env_logger::init();

    let _envs = dotenv::dotenv().expect("Failed to load .env file");

    let shutdown_flag_control = ShutdownFlag::init();
    let shutdown_flag_indexer = shutdown_flag_control.clone();

    ctrlc::set_handler(move || {
        warn!("Ctrl+C received! Signaling worker to stop...");
        shutdown_flag_control.set_on();
    })
    .expect("Error setting Ctrl+C handler");

    let store = CachedBlockStore::new("/Users/illuque/tmp/")
        .expect("Failed to create CachedKeyValueStore (unrecoverable)");

    // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
    let alloy_provider =
        AlloyProvider::new(WS_URL).expect("Failed to create AlloyProvider (unrecoverable)");

    let indexer = Indexer::new(store, alloy_provider, INITIAL_BLOCK_HASH_ENV);

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
