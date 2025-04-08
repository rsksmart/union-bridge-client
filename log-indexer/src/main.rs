use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::{rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash};
use log::{error, info};
use log_indexer::config::Config;
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use rsk_provider::rpc::AlloyProvider;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

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
                .help("Sets the path to the configuration directory")
                .default_value("../config/local"), // for local usage within the crate
        )
        .get_matches();

    let default_logger = format!("{}/log4rs.yaml", CARGO_MANIFEST_DIR);
    let logger_path: &str = matches
        .get_one::<String>(LOGGER_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_logger);
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let default_config = format!("{}/../config/local", CARGO_MANIFEST_DIR);
    let config_path: &str = matches
        .get_one::<String>(CONFIG_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_config);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    let store = RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))?;

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .expect(&format!(
            "Invalid initial block hash: {}",
            config.indexer.initial_block_hash
        ));

    let indexer = LogIndexer::new(
        store,
        alloy_provider,
        initial_block_hash,
        config.load_managed_contracts(),
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
