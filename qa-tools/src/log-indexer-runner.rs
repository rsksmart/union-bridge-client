use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;

use common::{
    config::Config, rsk_indexer::RskIndexer, shutdown_flag::ShutdownFlag, types::BlockHash,
};
mod helpers;
use helpers::{get_block_hash, get_latest_block_hex, update_file_text, update_initial_block_hash};
use log_indexer::{indexer::LogIndexer, store::RawLogStore};
use rsk_provider::rpc::AlloyProvider;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    // Use block finality (number).
    #[arg(short = 'f')]
    block_finality: Option<u64>,

    // Use a block height (number); cannot provide both -f and -b.
    #[arg(short = 'b')]
    block_height: Option<u64>,

    // Cache size override (e.g. 500); updates the config file.
    #[arg(short = 'a')]
    cache_size: Option<u64>,

    // Whether to copy from the default config (true) or expect an existing config (false)
    #[arg(short = 'c', default_value = "true")]
    from_original_config: bool,

    // Environment: "dev" or "stage" (default: stage).
    #[arg(short = 'e', default_value = "stage")]
    env: String,

    // Mandatory tag (e.g. "happy_path").
    #[arg(short = 't')]
    tag: String,
}

const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";
const WEBSOCKET_ENDPOINT: &str = "ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket";

fn main() -> Result<()> {
    let args = Args::parse();

    if args.tag.is_empty() {
        return Err(anyhow!("Error: -t <tag> is mandatory."));
    }
    if args.block_finality.is_some() && args.block_height.is_some() {
        return Err(anyhow!(
            "Cannot provide both block finality (-f) and block height (-b)."
        ));
    }

    // Select the source configuration path based on environment.
    let source_config_path = if args.env == "dev" {
        "config/dev"
    } else {
        "config/stage"
    };

    let source_storage_folder = "/tmp/monitor-executions/default/storage";
    let source_config_file = format!("{}/config.yaml", source_config_path);
    let source_log_folder = "logs"; // (without trailing slash)
    let source_log_config_file = "log4rs.yaml";

    // Build target paths using the provided tag.
    let target_folder = format!("{}/{}", ROOT_DIRECTORY, args.tag);
    fs::create_dir_all(&target_folder)
        .with_context(|| format!("Creating target folder: {}", target_folder))?;

    let target_storage_folder = format!("{}/storage", target_folder);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_config_file = format!("{}/config.yaml", target_config_folder);
    let target_log_folder = target_folder.clone(); // same as target folder
    let target_log_config_file = format!("{}/log4rs.yaml", target_folder);

    // --- Handle log4rs.yaml: copy and update the log folder path.
    fs::create_dir_all(&target_log_folder)
        .with_context(|| format!("Creating target log folder: {}", target_log_folder))?;
    fs::copy(source_log_config_file, &target_log_config_file)
        .with_context(|| "Copying log config file")?;
    update_file_text(
        &target_log_config_file,
        source_log_folder,
        &target_log_folder,
    )?;

    // --- Handle config.yaml: copy it if requested.
    if args.from_original_config {
        fs::create_dir_all(&target_config_folder)
            .with_context(|| format!("Creating target config folder: {}", target_config_folder))?;
        fs::copy(&source_config_file, &target_config_file).with_context(|| {
            format!(
                "Copying config from {} to {}",
                source_config_file, target_config_file
            )
        })?;
        println!(
            "Copied config from {} to {}",
            source_config_file, target_config_file
        );
    } else {
        println!(
            "Not copying config; expecting existing config file at {}",
            target_config_file
        );
    }

    // --- Update initial_block_hash in the config file if -f or -b is provided.
    if let Some(finality) = args.block_finality {
        // Retrieve the latest block number from the provider.
        let latest_block_hex = get_latest_block_hex(WEBSOCKET_ENDPOINT)?;
        println!("Latest block (hex): {}", latest_block_hex);
        let latest_block_dec = u64::from_str_radix(&latest_block_hex.trim_start_matches("0x"), 16)
            .with_context(|| "Parsing latest block hex")?;
        println!("Latest block (decimal): {}", latest_block_dec);

        // Compute the target block (subtract finality).
        let target_block_dec = latest_block_dec.saturating_sub(finality);
        println!("Target block (decimal): {}", target_block_dec);
        let target_block_hex = format!("0x{:x}", target_block_dec);
        println!("Target block (hex): {}", target_block_hex);

        // Retrieve the block hash for the target block.
        let block_hash = get_block_hash(WEBSOCKET_ENDPOINT, &target_block_hex)?;
        println!(
            "Retrieved block hash using finality {}: {}",
            finality, block_hash
        );
        update_initial_block_hash(&target_config_file, &block_hash)?;
    } else if let Some(height) = args.block_height {
        let target_block_hex = format!("0x{:x}", height);
        println!(
            "Using block height {} converted to hex: {}",
            height, target_block_hex
        );
        let block_hash = get_block_hash(WEBSOCKET_ENDPOINT, &target_block_hex)?;
        println!("Retrieved block hash for height {}: {}", height, block_hash);
        update_initial_block_hash(&target_config_file, &block_hash)?;
    } else {
        println!("No block finality or block height provided. Using existing initial_block_hash in config.");
    }

    // --- Update cache size if provided.
    if let Some(cache) = args.cache_size {
        update_file_text(
            &target_config_file,
            "size: 1000",
            &format!("size: {}", cache),
        )?;
        println!("Updated cache size to {} in {}", cache, target_config_file);
    }

    // --- Update storage path in the config file.
    update_file_text(
        &target_config_file,
        source_storage_folder,
        &target_storage_folder,
    )?;
    println!(
        "Updated storage folder in {} from {} to {}",
        target_config_file, source_storage_folder, target_storage_folder
    );

    // --- Run log-indexer by calling its library code.
    run_log_indexer(&target_log_config_file, &target_config_folder)?;

    Ok(())
}

// Calls log-indexer's library code.
fn run_log_indexer(log_config_path: &str, config_folder: &str) -> Result<()> {
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;
    let config = Config::load(config_folder)
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
        config.load_managed_contracts(false),
        shutdown_flag,
    )
    .with_context(|| "Failed to create LogIndexer")?;
    indexer.run().inspect_err(|e| {
        log::error!("Error running log-indexer: {:?}", e);
    })?;
    Ok(())
}
