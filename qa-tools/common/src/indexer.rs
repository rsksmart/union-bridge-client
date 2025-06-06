use anyhow::{Context, Result, anyhow};
use std::fs;

use crate::common::{
    config_consts, get_block_hash, get_latest_block_hex, update_file_text,
    update_initial_block_hash,
};

pub mod indexer_runner_args {
    use clap::{ArgAction, Parser};
    #[derive(Parser, Debug)]
    #[command(author, version, about)]
    pub struct Args {
        // Use block finality (number).
        #[arg(short = 'f')]
        pub block_finality: Option<u64>,

        // Use a block height (number); cannot provide both -f and -b.
        #[arg(short = 'b')]
        pub block_height: Option<u64>,

        // Cache size override (e.g. 500)
        #[arg(short = 'a')]
        pub cache_size: Option<u64>,

        // Whether to copy from the default config (true) or expect an existing config (false)
        #[arg(short = 'c', action = ArgAction::SetTrue)]
        pub use_existing_config: bool,

        // Environment: "dev" or "qa" (default: qa).
        #[arg(short = 'e', default_value = "qa")]
        pub env: String,

        // Mandatory tag (e.g. "happy_path").
        #[arg(short = 't')]
        pub tag: String,
    }
}

pub fn check_constraints(args: &indexer_runner_args::Args) -> Option<Result<(), anyhow::Error>> {
    if args.tag.is_empty() {
        return Some(Err(anyhow!("Error: -t <tag> is mandatory.")));
    }
    if args.block_finality.is_some() && args.block_height.is_some() {
        return Some(Err(anyhow!(
            "Cannot provide both block finality (-f) and block height (-b)."
        )));
    }
    None
}

#[derive(Clone)]
pub struct IndexerRunnerPaths {
    pub source_storage_folder: String,
    pub source_config_file: String,
    pub source_log_folder: &'static str,
    pub source_log_config_file: String,
    pub target_storage_folder: String,
    pub target_config_folder: String,
    pub target_config_file: String,
    pub target_log_folder: String,
    pub target_log_config_file: String,
}

pub fn set_paths(
    crate_name: String,
    args: &indexer_runner_args::Args,
) -> Result<IndexerRunnerPaths> {
    let source_config_path = if args.env == "dev" {
        format!("{}/config/dev", crate_name)
    } else {
        format!("{}/config/qa", crate_name)
    };
    let source_storage_folder = format!("{}/default/storage", config_consts::ROOT_DIRECTORY);
    let source_config_file = format!("{}/base_config.yaml", source_config_path);
    let source_log_folder = "{DESTINATION}/{CRATE_NAME}";
    let source_log_config_file = format!("{}/log4rs.yaml", source_config_path);
    let target_folder = format!("{}/{}", config_consts::ROOT_DIRECTORY, args.tag);
    fs::create_dir_all(&target_folder)
        .with_context(|| format!("Creating target folder: {}", target_folder))?;
    let target_storage_folder = format!("{}/storage", target_folder);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_config_file = format!("{}/common.yaml", target_config_folder);
    let target_log_folder = target_folder.clone();
    let target_log_config_file = format!("{}/log4rs.yaml", target_folder);
    Ok(IndexerRunnerPaths {
        source_storage_folder,
        source_config_file,
        source_log_folder,
        source_log_config_file,
        target_storage_folder,
        target_config_folder,
        target_config_file,
        target_log_folder,
        target_log_config_file,
    })
}

pub fn update_initial_block_hash_in_config(
    args: &indexer_runner_args::Args,
    target_config_file: &String,
    endpoint_url: &str,
) -> Result<(), anyhow::Error> {
    Ok(if let Some(finality) = args.block_finality {
        let latest_block_hex = get_latest_block_hex(endpoint_url)?;
        println!("Latest block (hex): {}", latest_block_hex);
        let latest_block_dec = u64::from_str_radix(&latest_block_hex.trim_start_matches("0x"), 16)
            .with_context(|| "Parsing latest block hex")?;
        println!("Latest block (decimal): {}", latest_block_dec);
        let target_block_dec = latest_block_dec.saturating_sub(finality);
        println!("Target block (decimal): {}", target_block_dec);
        let target_block_hex = format!("0x{:x}", target_block_dec);
        println!("Target block (hex): {}", target_block_hex);
        let block_hash = get_block_hash(endpoint_url, &target_block_hex)?;
        println!(
            "Retrieved block hash using finality {}: {}",
            finality, block_hash
        );
        update_initial_block_hash(target_config_file, &block_hash)?;
    } else if let Some(height) = args.block_height {
        let target_block_hex = format!("0x{:x}", height);
        println!(
            "Using block height {} converted to hex: {}",
            height, target_block_hex
        );
        let block_hash = get_block_hash(endpoint_url, &target_block_hex)?;
        println!("Retrieved block hash for height {}: {}", height, block_hash);
        update_initial_block_hash(target_config_file, &block_hash)?;
    } else {
        println!(
            "No block finality or block height provided. Using existing initial_block_hash in config."
        );
    })
}

pub fn update_cache_size_in_config(
    args: &indexer_runner_args::Args,
    target_config_file: &String,
) -> Result<(), anyhow::Error> {
    Ok(if let Some(cache) = args.cache_size {
        update_file_text(
            target_config_file,
            "size: 1000",
            &format!("size: {}", cache),
        )?;
        println!("Updated cache size to {} in {}", cache, target_config_file);
    })
}

pub fn update_storage_path_in_config(
    source_storage_folder: String,
    target_storage_folder: String,
    target_config_file: String,
) -> Result<(), anyhow::Error> {
    update_file_text(
        &target_config_file,
        &source_storage_folder,
        &target_storage_folder,
    )?;
    println!(
        "Updated storage folder in {} from {} to {}",
        target_config_file, source_storage_folder, target_storage_folder
    );
    Ok(())
}
