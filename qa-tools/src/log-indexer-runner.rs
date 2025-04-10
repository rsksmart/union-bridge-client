use anyhow::{Context, Result};
use clap::Parser;
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::{rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash};
use log_indexer::{config::Config as LogConfig, indexer::LogIndexer, store::RawLogStore};
use qa_tools::utils::common::{check_constraints, indexer_args};
use qa_tools::utils::runner::{
    copy_config_file, copy_log4rs_file, set_paths, update_cache_size_in_config,
    update_initial_block_hash_in_config, update_storage_path_in_config,
};

fn main() -> Result<()> {
    let args = indexer_args::Args::parse();
    if let Some(err) = check_constraints(&args) {
        return err;
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
    ) = set_paths("log-indexer".to_string(), &args)?;
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
    run_log_indexer(&target_log_config_file, &target_config_folder)?;
    Ok(())
}

// Calls log-indexer's library code.
fn run_log_indexer(log_config_path: &str, config_folder: &str) -> Result<()> {
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;
    let config = LogConfig::load(Some(&config_folder.to_string()))
        .with_context(|| format!("Loading config from {}", config_folder))?;
    let store = RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))
        .with_context(|| "Creating log-indexer store")?;
    let shutdown_flag = ShutdownFlag::init();
    let alloy_provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .with_context(|| "Creating AlloyProvider")?;
    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .with_context(|| "Parsing initial block hash")?;
    let indexer = LogIndexer::new(
        store,
        alloy_provider,
        initial_block_hash,
        config.load_managed_contracts(),
        shutdown_flag,
    )
    .with_context(|| "Failed to create LogIndexer")?;
    indexer.run().inspect_err(|e| {
        log::error!("Error running log-indexer: {:?}", e);
    })?;
    Ok(())
}
