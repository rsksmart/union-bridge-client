use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::{config::Config, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag};
use log::{error, info};
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use rsk_provider::rpc::AlloyProvider;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Log Indexer")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file")
                .default_value("../log4rs.yaml"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory")
                .default_value("../config/dev/config.yaml"),
        )
        .get_matches();

    let logger_path: &String = matches.get_one(LOGGER_CLI_FLAG).unwrap();
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let config_path: &String = matches.get_one(CONFIG_CLI_FLAG).unwrap();
    let config = Config::load(config_path).expect("Failed to load config");

    let store = RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))?;

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let indexer = LogIndexer::new(
        store,
        alloy_provider,
        &config.indexer.initial_block_hash,
        config.load_contracts(),
        shutdown_flag,
    )
    .context("Failed to create LogIndexer")?;

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running log indexer: {:?}", e);
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
