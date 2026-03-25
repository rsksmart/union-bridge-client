use std::sync::mpsc;

use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::msg_broker::broker::BrokerServer;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::types::RskLog;
use log::{debug, error, info};
use log_indexer::config::{Config, Logger};
use log_indexer::indexer::LogIndexer;
use log_indexer::notifier::Notifier;
use log_indexer::store::RawLogStore;

const LOGGER_CLI_FLAG: &str = "logger-path";
const ENV_CLI_FLAG: &str = "env";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Log Indexer")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(ENV_CLI_FLAG)
                .short('e')
                .long(ENV_CLI_FLAG)
                .value_name("ENV")
                .help("Environment name (e.g., local, alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>(ENV_CLI_FLAG).cloned();

    info!(
        "Loading configuration for environment: {}",
        env_name.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(env_name).expect("Failed to load config");

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

    let mut notifier = Notifier::new(
        rx,
        BrokerServer::new(
            config.log_indexer_config.notifier.port,
            &config.log_indexer_config.broker_key_path,
        )
        .expect("Failed to create BrokerServer"),
        monitored_addresses,
        shutdown_flag.clone(),
    );

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
    log::logger().flush();

    Ok(())
}
