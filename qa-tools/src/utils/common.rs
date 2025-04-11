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
        #[arg(short = 'c', default_value = "true")]
        pub from_original_config: bool,

        // Environment: "dev" or "qa" (default: qa).
        #[arg(short = 'e', default_value = "qa")]
        pub env: String,

        // Mandatory tag (e.g. "happy_path").
        #[arg(short = 't')]
        pub tag: String,
    }
}

pub mod indexer_consts {
    pub const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";
    pub const WEBSOCKET_ENDPOINT: &str = "ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket";
}

#[derive(Clone)]
pub struct RunnerPaths {
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

pub fn check_constraints(args: &indexer_args::Args) -> Option<Result<(), anyhow::Error>> {
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

pub fn update_file_text<P: AsRef<Path>>(path: P, from: &str, to: &str) -> Result<()> {
    let content =
        fs::read_to_string(&path).with_context(|| format!("Reading file {:?}", path.as_ref()))?;
    let new_content = content.replace(from, to);
    fs::write(&path, new_content).with_context(|| format!("Writing file {:?}", path.as_ref()))?;
    Ok(())
}

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
