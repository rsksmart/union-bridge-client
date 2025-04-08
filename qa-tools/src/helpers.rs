use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::{fs, path::Path};
use tungstenite::{connect, Message};
use url::Url;

pub mod indexer_args {
    use clap::Parser;
    #[derive(Parser, Debug)]
    #[command(author, version, about)]
    pub(crate) struct Args {
        // Use block finality (number).
        #[arg(short = 'f')]
        pub(crate) block_finality: Option<u64>,

        // Use a block height (number); cannot provide both -f and -b.
        #[arg(short = 'b')]
        pub(crate) block_height: Option<u64>,

        // Cache size override (e.g. 500); updates the config file.
        #[arg(short = 'a')]
        pub(crate) cache_size: Option<u64>,

        // Whether to copy from the default config (true) or expect an existing config (false)
        #[arg(short = 'c', default_value = "true")]
        pub(crate) from_original_config: bool,

        // Environment: "dev" or "stage" (default: stage).
        #[arg(short = 'e', default_value = "stage")]
        pub(crate) env: String,

        // Mandatory tag (e.g. "happy_path").
        #[arg(short = 't')]
        pub(crate) tag: String,
    }
}

mod indexer_consts {
    pub(crate) const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";

    pub(crate) const WEBSOCKET_ENDPOINT: &str =
        "ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket";
}

pub fn check_constraints(
    args: &indexer_args::Args,
) -> Option<std::result::Result<(), anyhow::Error>> {
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

pub fn set_paths(
    args: &indexer_args::Args,
) -> Result<
    (
        &str,
        String,
        &str,
        &str,
        String,
        String,
        String,
        String,
        String,
    ),
    anyhow::Error,
> {
    let source_config_path = if args.env == "dev" {
        "config/dev"
    } else {
        "config/stage"
    };
    let source_storage_folder = "/tmp/monitor-executions/default/storage";
    let source_config_file = format!("{}/config.yaml", source_config_path);
    let source_log_folder = "logs";
    let source_log_config_file = "log4rs.yaml";
    let target_folder = format!("{}/{}", indexer_consts::ROOT_DIRECTORY, args.tag);
    fs::create_dir_all(&target_folder)
        .with_context(|| format!("Creating target folder: {}", target_folder))?;
    let target_storage_folder = format!("{}/storage", target_folder);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_config_file = format!("{}/config.yaml", target_config_folder);
    let target_log_folder = target_folder.clone();
    let target_log_config_file = format!("{}/log4rs.yaml", target_folder);
    Ok((
        source_storage_folder,
        source_config_file,
        source_log_folder,
        source_log_config_file,
        target_storage_folder,
        target_config_folder,
        target_config_file,
        target_log_folder,
        target_log_config_file,
    ))
}

pub fn copy_log4rs_file(
    source_log_folder: &str,
    source_log_config_file: &str,
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
    Ok(if args.from_original_config {
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
        // Retrieve the latest block number from the provider.
        let latest_block_hex = get_latest_block_hex(indexer_consts::WEBSOCKET_ENDPOINT)?;
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
    source_storage_folder: &str,
    target_storage_folder: String,
    target_config_file: String,
) -> Result<(), anyhow::Error> {
    update_file_text(
        &target_config_file,
        source_storage_folder,
        &target_storage_folder,
    )?;
    println!(
        "Updated storage folder in {} from {} to {}",
        target_config_file, source_storage_folder, target_storage_folder
    );
    Ok(())
}

/// Reads a file, replaces all occurrences of `from` with `to`, and writes it back.
pub fn update_file_text<P: AsRef<Path>>(path: P, from: &str, to: &str) -> Result<()> {
    let content =
        fs::read_to_string(&path).with_context(|| format!("Reading file {:?}", path.as_ref()))?;
    let new_content = content.replace(from, to);
    fs::write(&path, new_content).with_context(|| format!("Writing file {:?}", path.as_ref()))?;
    Ok(())
}

/// Uses a regex substitution to update the `initial_block_hash` field in the config file.
pub fn update_initial_block_hash<P: AsRef<Path>>(path: P, block_hash: &str) -> Result<()> {
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Reading config file {:?}", path.as_ref()))?;
    let re = Regex::new(r#"(initial_block_hash:\s*")[^"]*(")"#)
        .with_context(|| "Compiling regex for initial_block_hash")?;
    let new_content = re
        .replace_all(&content, format!("${{1}}{}${{2}}", block_hash))
        .to_string();
    fs::write(&path, new_content)
        .with_context(|| format!("Writing updated config file {:?}", path.as_ref()))?;
    Ok(())
}

/// Connects to the WebSocket endpoint and retrieves the latest block number (hex).
pub fn get_latest_block_hex(endpoint: &str) -> Result<String> {
    let (mut socket, _) = connect(Url::parse(endpoint)?.to_string())
        .with_context(|| format!("Connecting to WebSocket endpoint: {}", endpoint))?;
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_blockNumber",
        "params": []
    });
    socket
        .send(Message::Text(req.to_string().into()))
        .with_context(|| "Sending eth_blockNumber request")?;
    let msg = socket
        .read()
        .with_context(|| "Reading eth_blockNumber response")?;
    let text = msg
        .into_text()
        .with_context(|| "Converting message to text")?;
    let json: Value = serde_json::from_str(&text)
        .with_context(|| "Parsing JSON from eth_blockNumber response")?;
    json.get("result")
        .and_then(|r| r.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("Missing block number result"))
}

/// Retrieves the block hash for the specified block (provided as a hex string).
pub fn get_block_hash(endpoint: &str, block_hex: &str) -> Result<String> {
    let (mut socket, _) = connect(Url::parse(endpoint)?.to_string())
        .with_context(|| format!("Connecting to WebSocket for block lookup: {}", endpoint))?;
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "eth_getBlockByNumber",
        "params": [block_hex, false]
    });
    socket
        .send(Message::Text(req.to_string().into()))
        .with_context(|| "Sending eth_getBlockByNumber request")?;
    let msg = socket
        .read()
        .with_context(|| "Reading eth_getBlockByNumber response")?;
    let text = msg
        .into_text()
        .with_context(|| "Converting block response to text")?;
    let json: Value =
        serde_json::from_str(&text).with_context(|| "Parsing JSON from block lookup")?;
    json.get("result")
        .and_then(|r| r.get("hash"))
        .and_then(|h| h.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("Missing block hash in response for block {}", block_hex))
}
