use anyhow::{Context, Result, anyhow};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::Path};
use tungstenite::{Message, connect};
use url::Url;

pub mod config_consts {
    pub const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";
}

pub fn copy_log4rs_file(
    source_log_folder: &str,
    source_log_config_file: String,
    target_log_folder: String,
    target_log_config_file: &String,
) -> Result<(), anyhow::Error> {
    println!(
        "Copying log4rs config from {} to {}",
        source_log_config_file, target_log_config_file
    );
    println!("Source log folder: {}", source_log_folder);
    println!("Target log folder: {}", target_log_folder);
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
    use_existing_config: bool,
    source_config_file: String,
    target_config_folder: &String,
    target_config_file: &String,
) -> Result<(), anyhow::Error> {
    Ok(if use_existing_config {
        println!(
            "Not copying config; expecting existing config file at {}",
            target_config_file
        );
    } else {
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
    })
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

#[derive(Debug, Deserialize)]
struct Config {
    provider: Provider,
}
#[derive(Debug, Deserialize)]
struct Provider {
    rootstock: Rootstock,
}
#[derive(Debug, Deserialize)]
struct Rootstock {
    url: String,
}

pub fn get_endpoint_url(config_file_path: &str) -> Result<String> {
    let contents = std::fs::read_to_string(config_file_path)?;
    let config: Config = serde_yaml::from_str(&contents)?;
    Ok(config.provider.rootstock.url)
}
