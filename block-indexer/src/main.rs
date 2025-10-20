use anyhow::Result;
use block_indexer::config::{Config, Logger};
use block_indexer::notifier::Notifier;
use block_indexer::{indexer::BlockIndexer, store::CachedBlockStore};
use clap::{Arg, Command};
use common::msg_broker::broker::BrokerServer;
use common::types::RskBlockAndUncles;
use common::{
    alloy_rsk_provider::rpc::AlloyProvider, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag,
    types::BlockHash,
};
use log::{debug, error, info};
use std::sync::mpsc;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

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
                .help("Environment name (e.g., docker-anvil, docker-alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>("env").cloned();

    info!("Loading configuration from environment config");
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

    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - think about bounding the channel
    let (tx, rx): (
        mpsc::Sender<RskBlockAndUncles>,
        mpsc::Receiver<RskBlockAndUncles>,
    ) = mpsc::channel();

    let store_path = &format!("{}/blocks", config.indexer.storage.path);
    debug!("Creating block store at: {}", store_path);
    let store = CachedBlockStore::new(store_path, config.indexer.cache.size)?;

    let indexer = BlockIndexer::new_with_notifier(
        store,
        alloy_provider,
        tx,
        initial_block_hash,
        shutdown_flag.clone(),
    );

    let mut notifier = Notifier::new(
        rx,
        BrokerServer::new(config.block_notifier.broker_port),
        shutdown_flag.clone(),
    );

    let shutdown_flag_notifier = shutdown_flag.clone();
    std::thread::spawn(move || {
        notifier.run().inspect_err(|e| {
            error!("Unrecoverable error running block notifier: {:?}", e);
            // signal other threads to shut down
            shutdown_flag_notifier.set();
        })
    });

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running block indexer: {:?}", e);
        // signal other threads to shut down
        shutdown_flag.set();
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
