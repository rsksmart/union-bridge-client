use anyhow::{Context, Result};
use clap::Parser;
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::{rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash};
use log_indexer::{config::Config as LogConfig, indexer::LogIndexer, store::RawLogStore};
use qa_tools_common::common::{copy_config_file, copy_log4rs_file, get_endpoint_url};
use qa_tools_common::indexer::{
    check_constraints, indexer_runner_args, set_paths, update_cache_size_in_config,
    update_initial_block_hash_in_config, update_storage_path_in_config,
};

fn main() -> Result<()> {
    let args = indexer_runner_args::Args::parse();
    if let Some(err) = check_constraints(&args) {
        return err;
    }
    let paths = set_paths("log-indexer".to_string(), &args)?;
    copy_log4rs_file(
        &paths.source_log_folder,
        paths.source_log_config_file,
        paths.target_log_folder,
        &paths.target_log_config_file,
    )?;
    copy_config_file(
        args.use_existing_config,
        paths.source_config_file,
        &paths.target_config_folder,
        &paths.target_config_file,
    )?;
    let endpoint_url = get_endpoint_url(&paths.target_config_file)?;
    update_initial_block_hash_in_config(&args, &paths.target_config_file, &endpoint_url)?;
    update_cache_size_in_config(&args, &paths.target_config_file)?;
    update_storage_path_in_config(
        paths.source_storage_folder,
        paths.target_storage_folder,
        paths.target_config_file,
    )?;
    run_log_indexer(&paths.target_log_config_file, &paths.target_config_folder)?;
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
        config.indexer.sync.batch_size,
        config.indexer.sync.finality_depth,
        config.load_managed_contracts(),
        shutdown_flag,
    )
    .with_context(|| "Failed to create LogIndexer")?;
    indexer.run().inspect_err(|e| {
        log::error!("Error running log-indexer: {:?}", e);
    })?;
    Ok(())
}
