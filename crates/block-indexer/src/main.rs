#![forbid(unsafe_code)]

use std::sync::mpsc;

use anyhow::{Context, Result, ensure};
use block_indexer::config::{Config, Logger};
use block_indexer::indexer::BlockIndexer;
use block_indexer::notifier::Notifier;
use block_indexer::store::CachedBlockStore;
use clap::{Arg, Command};
use common_broker::broker::{BrokerServer, Identifier, broker_queue_storage_path};
use common_core::types::RskBlockAndUncles;
use common_rsk::alloy_rsk_provider::rpc::AlloyProvider;
use common_rsk::rsk_indexer::RskIndexer;
use common_runtime::shutdown_flag::ShutdownFlag;
use tracing::{debug, error, info};

const LOG_DIR_CLI_FLAG: &str = "log-dir";
const CONFIG_CLI_FLAG: &str = "config";
const BROKER_QUEUE_SERVICE_NAME: &str = "block-indexer";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
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

    let coordinator_pubkey_hash = config.block_indexer_config.coordinator.pubkey_hash.clone();
    ensure!(
        !coordinator_pubkey_hash.is_empty() && coordinator_pubkey_hash != "<to_patch_with_env>",
        "block_indexer.coordinator.pubkey_hash must be configured"
    );
    let coordinator_identifier = Identifier::new(
        coordinator_pubkey_hash,
        u8::try_from(config.block_indexer_config.coordinator.client_id)
            .context("block_indexer.coordinator.client_id must fit in u8")?,
    );

    let broker_server = BrokerServer::new_with_storage_path(
        config.block_indexer_config.notifier.port,
        &config.block_indexer_config.broker_key_path,
        broker_queue_storage_path(&config.indexer.storage.path, BROKER_QUEUE_SERVICE_NAME),
        &coordinator_identifier,
    )
    .context("Failed to create BrokerServer")?;

    let mut notifier = Notifier::new(rx, broker_server, shutdown_flag.clone());

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

    Ok(())
}
