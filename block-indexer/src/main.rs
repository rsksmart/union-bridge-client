use anyhow::Result;
use block_indexer::{indexer::BlockIndexer, store::CachedBlockStore};
use clap::{Arg, Command};
use common::{config::Config, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag};
use log::info;
use rsk_provider::rpc::AlloyProvider;

fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
        .arg(
            Arg::new("logger-path")
                .short('l')
                .long("logger-path")
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new("config-path")
                .short('c')
                .long("config-path")
                .value_name("PATH")
                .help("Sets the path to the configuration directory"),
        )
        .get_matches();


    let logger_path: &String = matches.get_one("logger-path").unwrap();
    log4rs::init_file(logger_path, Default::default())
        .expect("Failed to load log4rs config");

    let config_path: &String = matches.get_one("config-path").unwrap();
    let config = Config::load(config_path).expect("Failed to load config");

    let store = CachedBlockStore::new(
        &format!("{}/blocks", config.indexer.storage.path),
        config.indexer.cache.size,
    )?;

    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .expect("Failed to create AlloyProvider (unrecoverable)");

    let indexer = BlockIndexer::new(
        store,
        alloy_provider,
        &config.indexer.initial_block_hash,
        shutdown_flag,
    );

    indexer.run().inspect_err(|e| {
        error!("Unrecoverable error running block indexer: {:?}", e);
    })?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
