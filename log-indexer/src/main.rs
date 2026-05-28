#![forbid(unsafe_code)]

use std::sync::mpsc;

use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::msg_broker::broker::{BrokerServer, broker_queue_storage_path};
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::types::RskLog;
use log_indexer::config::{Config, Logger};
use log_indexer::indexer::LogIndexer;
use log_indexer::notifier::Notifier;
use log_indexer::store::RawLogStore;
use tracing::{debug, error, info};

const LOG_DIR_CLI_FLAG: &str = "log-dir";
const CONFIG_CLI_FLAG: &str = "config";
const BROKER_QUEUE_SERVICE_NAME: &str = "log-indexer";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Log Indexer")
        .arg(Arg::new(LOG_DIR_CLI_FLAG).short('l').long(LOG_DIR_CLI_FLAG).value_name("DIR").help(
            "Directory for log files (also set via UB_LOG_DIR). Defaults to ./logs/ when unset.",
        ))
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('e')
                .long(CONFIG_CLI_FLAG)
                .value_name("CONFIG")
                .help("Configuration profile name (e.g., local, docker, alphanet)"),
        )
        .get_matches();

    let log_dir = matches.get_one::<String>(LOG_DIR_CLI_FLAG);
    let _log_guard = Logger::init(log_dir).expect("Failed to load logger");

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

    let (tx, rx): (mpsc::Sender<RskLog>, mpsc::Receiver<RskLog>) = mpsc::channel();

    let store_path = &format!("{}/logs", config.indexer.storage.path);
    debug!("Creating log store at: {store_path}");
    let store = RawLogStore::new(store_path)?;

    let managed_contracts = config.load_managed_contracts();
    let monitored_addresses = managed_contracts.keys().copied().collect();

    let indexer = LogIndexer::new_with_notifier(
        store,
        alloy_provider,
        tx,
        &config.indexer,
        managed_contracts,
        shutdown_flag.clone(),
    )
    .context("Failed to create LogIndexer")?;

    let broker_server = BrokerServer::new_with_storage_path(
        config.log_indexer_config.notifier.port,
        &config.log_indexer_config.broker_key_path,
        &broker_queue_storage_path(&config.indexer.storage.path, BROKER_QUEUE_SERVICE_NAME),
    )
    .context("Failed to create BrokerServer")?;

    let mut notifier = Notifier::new(rx, broker_server, monitored_addresses, shutdown_flag.clone());

    let shutdown_flag_notifier = shutdown_flag.clone();
    std::thread::spawn(move || {
        notifier.run().inspect_err(|e| {
            error!("Unrecoverable error running log notifier: {e:?}");
            // signal other threads to shut down
            shutdown_flag_notifier.set();
        })
    });

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running log indexer: {e:?}");
    })?;

    info!("Quitting now...");

    Ok(())
}
