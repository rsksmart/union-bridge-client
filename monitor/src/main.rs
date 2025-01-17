use anyhow::{anyhow, Result};
use log::warn;
use monitor::indexer::Indexer;
use monitor::store::CachedKeyValueStore;
use monitor::utils::ShutdownFlag;
use std::thread;

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const WS_URL: &str = "wss://public-node.testnet.rsk.co/websocket";

// TODO(Jira) move to .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK_HASH_ENV: &str =
    "0x9f671f86e4e8f9ee802ba7224d99caa7771f5f4a723db53590f2b693d66eb621";

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

    let store = CachedKeyValueStore::new("/Users/illuque/tmp/")
        .expect("Failed to create CachedKeyValueStore");
    let indexer = Indexer::new(store, WS_URL, INITIAL_BLOCK_HASH_ENV);

    run_indexer(indexer, shutdown_flag_indexer)?;

    log::logger().flush();

    Ok(())
}

fn run_indexer(indexer: Indexer, shutdown_flag: ShutdownFlag) -> Result<()> {
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
