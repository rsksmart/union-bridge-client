use anyhow::{Context, Result};
use std::fs;

use crate::utils::common::{
    get_block_hash, get_latest_block_hex, indexer_args, indexer_consts, update_file_text,
    update_initial_block_hash, RunnerPaths,
};

pub fn set_paths(crate_name: String, args: &indexer_args::Args) -> Result<RunnerPaths> {
    let source_config_path = if args.env == "dev" {
        format!("qa-tools/config/dev/{}", crate_name)
    } else {
        format!("qa-tools/config/qa/{}", crate_name)
    };
    let source_storage_folder = format!("{}/default/storage", indexer_consts::ROOT_DIRECTORY);
    let source_config_file = format!("{}/base_config.yaml", source_config_path);
    let source_log_folder = "{DESTINATION}/{CRATE_NAME}";
    let source_log_config_file = format!("{}/log4rs.yaml", source_config_path);
    let target_folder = format!("{}/{}", indexer_consts::ROOT_DIRECTORY, args.tag);
    fs::create_dir_all(&target_folder)
        .with_context(|| format!("Creating target folder: {}", target_folder))?;
    let target_storage_folder = format!("{}/storage", target_folder);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_config_file = format!("{}/common.yaml", target_config_folder);
    let target_log_folder = target_folder.clone();
    let target_log_config_file = format!("{}/log4rs.yaml", target_folder);
    Ok(RunnerPaths {
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

pub fn copy_log4rs_file(
    source_log_folder: &str,
    source_log_config_file: String,
    target_log_folder: String,
    target_log_config_file: &String,
) -> Result<(), anyhow::Error> {
    fs::create_dir_all(&target_log_folder)
        .with_context(|| format!("Creating target log folder: {}", target_log_folder))?;
    fs::copy(source_log_config_file, target_log_config_file)
        .with_context(|| "Copying log config file")?;
    update_file_text(
        target_log_config_file,
        source_log_folder,
        &target_log_folder,
    )?;
    Ok(())
}

pub fn copy_config_file(
    args: &indexer_args::Args,
    source_config_file: String,
    target_config_folder: &String,
    target_config_file: &String,
) -> Result<(), anyhow::Error> {
    Ok(if args.from_original_config.unwrap_or(true) {
        fs::create_dir_all(target_config_folder)
            .with_context(|| format!("Creating target config folder: {}", target_config_folder))?;
        fs::copy(&source_config_file, target_config_file).with_context(|| {
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
    })
}

pub fn update_initial_block_hash_in_config(
    args: &indexer_args::Args,
    target_config_file: &String,
) -> Result<(), anyhow::Error> {
    Ok(if let Some(finality) = args.block_finality {
        let latest_block_hex = get_latest_block_hex(indexer_consts::WEBSOCKET_ENDPOINT)?;
        println!("Latest block (hex): {}", latest_block_hex);
        let latest_block_dec = u64::from_str_radix(&latest_block_hex.trim_start_matches("0x"), 16)
            .with_context(|| "Parsing latest block hex")?;
        println!("Latest block (decimal): {}", latest_block_dec);
        let target_block_dec = latest_block_dec.saturating_sub(finality);
        println!("Target block (decimal): {}", target_block_dec);
        let target_block_hex = format!("0x{:x}", target_block_dec);
        println!("Target block (hex): {}", target_block_hex);
        let block_hash = get_block_hash(indexer_consts::WEBSOCKET_ENDPOINT, &target_block_hex)?;
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
        let block_hash = get_block_hash(indexer_consts::WEBSOCKET_ENDPOINT, &target_block_hex)?;
        println!("Retrieved block hash for height {}: {}", height, block_hash);
        update_initial_block_hash(target_config_file, &block_hash)?;
    } else {
        println!("No block finality or block height provided. Using existing initial_block_hash in config.");
    })
}

pub fn update_cache_size_in_config(
    args: &indexer_args::Args,
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
