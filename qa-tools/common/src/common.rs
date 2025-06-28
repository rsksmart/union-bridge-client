use anyhow::{Context, Result, anyhow};
use cucumber::gherkin::Step;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Result as ioResult;
use std::process::{Child, Command, Output};
use std::thread::sleep;
use std::time::Duration;
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
    let contents = fs::read_to_string(config_file_path)?;
    let config: Config = serde_yaml::from_str(&contents)?;
    Ok(config.provider.rootstock.url)
}

pub fn spawn_command(command: &str) -> Child {
    Command::new("bash")
        .arg("-c")
        .arg(command)
        .spawn()
        .unwrap_or_else(|e| panic!("Failed to spawn `{}`: {}", command, e))
}

pub fn execute_command(command: &str) {
    let output = execute_command_output(command)
        .unwrap_or_else(|e| panic!("Failed to execute `{}`: {}", command, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "`{}` failed (status: {}):\n{}",
            command, output.status, stderr
        );
    }
}

fn execute_command_output(command: &str) -> ioResult<Output> {
    Command::new("bash").arg("-c").arg(command).output()
}

pub fn execute_script(script_path: &str) {
    execute_command(&format!("chmod +x {}", script_path));
    let output = Command::new("bash").arg(script_path).output()
        .unwrap_or_else(|e| panic!("Failed to execute `{}`: {}", script_path, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "`{}` failed (status: {}):\n{}",
            script_path, output.status, stderr
        );
    }
}

fn send_sigterm(pid: u32) {
    execute_command(&format!("kill -15 {}", pid));
}

fn wait_for_exit(child: &mut Child, timeout_ms: u64) -> ioResult<bool> {
    let mut waited = 0;
    while waited < timeout_ms {
        if let Some(_) = child.try_wait()? {
            return Ok(true);
        }
        sleep(Duration::from_millis(100));
        waited += 100;
    }
    Ok(false)
}

pub fn kill_process(child: &mut Child) {
    let pid = child.id();
    send_sigterm(pid);
    if !wait_for_exit(child, 3_000).unwrap_or(false) {
        child.kill().unwrap();
        if !wait_for_exit(child, 3_000).unwrap_or(false) {
            panic!("Failed to terminate process with PID: {}", pid);
        }
    }
}

pub fn extract_params(step: &Step) -> HashMap<String, String> {
    step.table
        .as_ref()
        .filter(|table| table.rows.len() == 2)
        .map(|table| {
            table.rows[0]
                .iter()
                .cloned()
                .zip(table.rows[1].iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}
