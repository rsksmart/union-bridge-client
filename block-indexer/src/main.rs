use std::sync::mpsc;

use anyhow::Result;
use block_indexer::config::{Config, Logger};
use block_indexer::indexer::BlockIndexer;
use block_indexer::notifier::Notifier;
use block_indexer::store::CachedBlockStore;
use clap::{Arg, Command};
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::msg_broker::broker::BrokerServer;
use common::rsk_indexer::RskIndexer;
use common::shutdown_flag::ShutdownFlag;
use common::types::{BlockHash, RskBlockAndUncles};
use log::{debug, error, info};

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
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_path = matches.get_one::<String>(CONFIG_CLI_FLAG);
    let config: Config = Config::load(config_path).expect("Failed to load config");

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
        BrokerServer::new(config.notifier.broker_port),
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
