use anyhow::{Context, Result};

use clap::Parser;
use common::{
    config::Config, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash,
};
mod helpers;
use block_indexer::{indexer::BlockIndexer, store::CachedBlockStore};
use helpers::{
    check_constraints, copy_config_file, copy_log4rs_file, indexer_args, set_paths,
    update_cache_size_in_config, update_initial_block_hash_in_config,
    update_storage_path_in_config,
};
use rsk_provider::rpc::AlloyProvider;

fn main() -> Result<()> {
    let args = indexer_args::Args::parse();
    if let Some(value) = check_constraints(&args) {
        return value;
    }
    let (
        source_storage_folder,
        source_config_file,
        source_log_folder,
        source_log_config_file,
        target_storage_folder,
        target_config_folder,
        target_config_file,
        target_log_folder,
        target_log_config_file,
    ) = set_paths(&args)?;
    copy_log4rs_file(
        source_log_folder,
        source_log_config_file,
        target_log_folder,
        &target_log_config_file,
    )?;
    copy_config_file(
        &args,
        source_config_file,
        &target_config_folder,
        &target_config_file,
    )?;
    update_initial_block_hash_in_config(&args, &target_config_file)?;
    update_cache_size_in_config(&args, &target_config_file)?;
    update_storage_path_in_config(
        source_storage_folder,
        target_storage_folder,
        target_config_file,
    )?;
    run_block_indexer(&target_log_config_file, &target_config_folder)?;
    Ok(())
}

// Calls block-indexer's library code.
fn run_block_indexer(log_config_path: &str, config_folder: &str) -> Result<()> {
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;
    let config = Config::load(config_folder)
        .with_context(|| format!("Loading config from {}", config_folder))?;
    let store = CachedBlockStore::new(
        &format!("{}/blocks", config.indexer.storage.path),
        config.indexer.cache.size,
    )
    .with_context(|| "Creating block store")?;
    let shutdown_flag = ShutdownFlag::init();

    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .with_context(|| "Creating AlloyProvider")?;
    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .with_context(|| "Parsing initial block hash")?;
    let indexer = BlockIndexer::new(store, alloy_provider, initial_block_hash, shutdown_flag);
    indexer.run().inspect_err(|e| {
        log::error!("Error running block-indexer: {:?}", e);
    })?;

    Ok(())
}
