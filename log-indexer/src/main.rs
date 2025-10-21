use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::BrokerServer;
use common::types::RskLog;
use common::{
    alloy_rsk_provider::rpc::AlloyProvider, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag,
    types::BlockHash,
};
use log::{debug, error, info};
use log_indexer::config::{Config, Logger};
use log_indexer::notifier::Notifier;
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use std::sync::mpsc;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

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
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory"),
        )
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .value_name("ENV")
                .help("Environment name (e.g., local, alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>("env").cloned();

    info!(
        "Loading configuration for environment: {}",
        env_name.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(env_name).expect("Failed to load config");

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .expect(&format!(
            "Invalid initial block hash: {}",
            config.indexer.initial_block_hash
        ));

    let (tx, rx): (mpsc::Sender<RskLog>, mpsc::Receiver<RskLog>) = mpsc::channel();

    let store_path = &format!("{}/logs", config.indexer.storage.path);
    debug!("Creating log store at: {}", store_path);
    let store = RawLogStore::new(store_path)?;

    let indexer = LogIndexer::new_with_notifier(
        store,
        alloy_provider,
        tx,
        initial_block_hash,
        config.indexer.sync.batch_size,
        config.indexer.sync.finality_depth,
        config.load_managed_contracts(),
        shutdown_flag.clone(),
    )
    .context("Failed to create LogIndexer")?;

    let mut notifier = Notifier::new(
        rx,
        BrokerServer::new(config.log_notifier.broker_port),
        shutdown_flag.clone(),
    );

    let shutdown_flag_notifier = shutdown_flag.clone();
    std::thread::spawn(move || {
        notifier.run().inspect_err(|e| {
            error!("Unrecoverable error running log notifier: {:?}", e);
            // signal other threads to shut down
            shutdown_flag_notifier.set();
        })
    });

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running log indexer: {:?}", e);
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
