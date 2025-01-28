use anyhow::Result;
use dotenv::dotenv;
use log::{info, warn};
use monitor::indexer::Indexer;
use monitor::rsk_provider::alloy::AlloyProvider;
use monitor::store::CachedBlockStore;
use monitor::utils::ShutdownFlag;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::{env, thread};

fn main() -> Result<()> {
    dotenv().expect("Failed to load .env file");

    log4rs::init_file("log4rs.yml", Default::default()).expect("Failed to load log4rs config");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let store = CachedBlockStore::new(&store_path)?;

    let rsk_url = env::var("RSK_PROVIDER_URL").expect("RSK_PROVIDER_URL not set in env");
    let alloy_provider =
        AlloyProvider::new(&rsk_url).expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    let shutdown_flag = ShutdownFlag::init();
    let indexer = Indexer::new(
        store,
        alloy_provider,
        &initial_block_hash,
        shutdown_flag.clone(),
    );

    let mut signals = Signals::new(&[SIGINT, SIGTERM]).expect("Failed to set up signal handlers");
    thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGINT => warn!("Received Ctrl+C!"),
                SIGTERM => warn!("Received SIGTERM!"),
                _ => unreachable!(),
            }
            shutdown_flag.set_on();
            break;
        }
    });

    indexer.run()?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
