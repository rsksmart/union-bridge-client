use anyhow::Result;
use block_indexer::{indexer::BlockIndexer, store::CachedBlockStore};
use clap::{Arg, Command};
use common::{
    config::Config, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash,
};
use log::{error, info};
use rsk_provider::rpc::AlloyProvider;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";
const STORAGE_CLI_FLAG: &str = "storage-path";
const INITIAL_BLOCK_HASH_CLI_FLAG: &str = "initial-block-hash";
const CACHE_SIZE_CLI_FLAG: &str = "cache-size";

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file")
                .default_value("log4rs.yaml"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory")
                .default_value("config/dev"),
        )
        .arg(
            Arg::new(STORAGE_CLI_FLAG)
                .short('s')
                .long(STORAGE_CLI_FLAG)
                .value_name("PATH")
                .help("Overrides the storage path for blocks"),
        )
        .arg(
            Arg::new(INITIAL_BLOCK_HASH_CLI_FLAG)
                .short('b')
                .long(INITIAL_BLOCK_HASH_CLI_FLAG)
                .value_name("HASH")
                .help("Overrides the initial block hash"),
        )
        .arg(
            Arg::new(CACHE_SIZE_CLI_FLAG)
                .short('a')
                .long(CACHE_SIZE_CLI_FLAG)
                .value_name("SIZE")
                .help("Overrides the cache size"),
        )
        .get_matches();

    let logger_path: &String = matches.get_one(LOGGER_CLI_FLAG).unwrap();
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let config_path: &String = matches.get_one(CONFIG_CLI_FLAG).unwrap();
    let config = Config::load(config_path).expect("Failed to load config");

    // Get storage path from CLI argument if provided, otherwise fallback to config.
    let storage_path = matches
        .get_one::<String>(STORAGE_CLI_FLAG)
        .unwrap_or(&config.indexer.storage.path)
        .clone();

    // Get cache size from CLI argument if provided, otherwise fallback to config.
    let cache_size = if let Some(cache_size_str) = matches.get_one::<String>(CACHE_SIZE_CLI_FLAG) {
        cache_size_str
            .parse::<usize>()
            .expect("Invalid cache size provided")
    } else {
        config.indexer.cache.size
    };

    // Get initial block hash from CLI argument if provided, otherwise fallback to config.
    let initial_block_hash_str = matches
        .get_one::<String>(INITIAL_BLOCK_HASH_CLI_FLAG)
        .unwrap_or(&config.indexer.initial_block_hash)
        .clone();
    let initial_block_hash = BlockHash::try_from(initial_block_hash_str.as_str()).expect(&format!(
        "Invalid initial block hash: {}",
        initial_block_hash_str
    ));

    let store = CachedBlockStore::new(&format!("{}/blocks", storage_path), cache_size)?;

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let indexer = BlockIndexer::new(store, alloy_provider, initial_block_hash, shutdown_flag);

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running block indexer: {:?}", e);
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
