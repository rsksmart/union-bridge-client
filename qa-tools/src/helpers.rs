use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::{fs, path::Path};
use tungstenite::{connect, Message};
use url::Url;

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
