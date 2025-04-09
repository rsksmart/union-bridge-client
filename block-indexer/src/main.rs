use anyhow::Result;
use block_indexer::config::{Config, Logger};
use block_indexer::{indexer::BlockIndexer, store::CachedBlockStore};
use clap::{Arg, Command};
use common::{
    alloy_rsk_provider::rpc::AlloyProvider, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag,
    types::BlockHash,
};
use log::{error, info};

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

    let store = CachedBlockStore::new(
        &format!("{}/blocks", config.indexer.storage.path),
        config.indexer.cache.size,
    )?;

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .expect(&format!(
            "Invalid initial block hash: {}",
            config.indexer.initial_block_hash
        ));

    let indexer = BlockIndexer::new(store, alloy_provider, initial_block_hash, shutdown_flag);

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running block indexer: {:?}", e);
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
