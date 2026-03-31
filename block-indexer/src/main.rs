use std::sync::mpsc;

use anyhow::{Context, Result};
use block_indexer::config::{Config, Logger};
use block_indexer::indexer::BlockIndexer;
use block_indexer::notifier::Notifier;
use block_indexer::store::CachedBlockStore;
use clap::{Arg, Command};
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::msg_broker::broker::BrokerServer;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::types::RskBlockAndUncles;
use log::{debug, error, info};

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('e')
                .long(CONFIG_CLI_FLAG)
                .value_name("CONFIG")
                .help("Configuration profile name (e.g., local, docker, alphanet)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_name = matches.get_one::<String>(CONFIG_CLI_FLAG).cloned();

    info!(
        "Loading configuration profile: {}",
        config_name.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(config_name).expect("Failed to load config");

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    // TODO(UB-132) think about bounding the channel
    let (tx, rx): (mpsc::Sender<RskBlockAndUncles>, mpsc::Receiver<RskBlockAndUncles>) =
        mpsc::channel();

    let store_path = &format!("{}/blocks", config.indexer.storage.path);
    debug!("Creating block store at: {store_path}");
    let store = CachedBlockStore::new(store_path, config.indexer.cache.size)?;

    let indexer = BlockIndexer::new_with_notifier(
        store,
        alloy_provider,
        tx,
        &config.indexer,
        shutdown_flag.clone(),
    )
    .context("Failed to create BlockIndexer")?;

    let mut notifier = Notifier::new(
        rx,
        BrokerServer::new(
            config.block_indexer_config.notifier.port,
            &config.block_indexer_config.broker_key_path,
        )
        .expect("Failed to create BrokerServer"),
        shutdown_flag.clone(),
    );

    let shutdown_flag_notifier = shutdown_flag.clone();
    std::thread::spawn(move || {
        notifier.run().inspect_err(|e| {
            error!("Unrecoverable error running block notifier: {e:?}");
            // signal other threads to shut down
            shutdown_flag_notifier.set();
        })
    });

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running block indexer: {e:?}");
        // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
