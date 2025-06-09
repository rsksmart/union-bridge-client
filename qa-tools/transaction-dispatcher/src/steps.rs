use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use byteorder::{LittleEndian, WriteBytesExt};
use cucumber::{World, gherkin::Step};
use hex;
use once_cell::sync::Lazy;
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

pub static SEEDED_RNG: Lazy<Mutex<StdRng>> = Lazy::new(|| Mutex::new(StdRng::seed_from_u64(45)));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcOutput {
    pub amount: u64,
    pub script_pub_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcInput {
    pub tx_id: String,
    pub v_out: u32,
    pub sequence: u32,
    pub script_sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcTransaction {
    pub version: u32,
    pub outputs: Vec<BtcOutput>,
    pub inputs: Vec<BtcInput>,
    pub lock_time: u32,
}

#[derive(Debug, Default, World)]
pub struct TestWorld {
    pub response: Option<String>,
    pub status_code: Option<u16>,
    pub response_2: Option<String>,
    pub status_code_2: Option<u16>,
    pub register_pegin_block_hash: Option<String>,
    pub register_pegin_btc_txid: Option<String>,
    pub register_pegin_merkle_hash: Option<String>,
}

pub const DEFAULT_SEQUENCE: u32 = 4294967293;
pub const DEFAULT_V_OUT: u32 = 1694;
pub const DEFAULT_SCRIPT_PUB_KEY_0: &str =
    "0x5120228f281f297fd01cd363b9c93f742ba2976c1ec5a6083d9f754cb61e505356c3";
pub const DEFAULT_SCRIPT_PUB_KEY_0_ACCEPT_PEGIN: &str =
    "0x51209687ca13c4fb3fa3ba05c2f9119dda026bfe66f0098dcf9b896a98ecb2e96702";
pub const DEFAULT_SCRIPT_PUB_KEY_1: &str = "0x6a4552534b5f504547494e000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c87d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f";
pub const DEFAULT_SCRIPT_PUB_KEY_1_ACCEPT_PEGIN: &str =
    "0x0014298a0fe992f755152a81ee64bdc4cc96d3bb8969";
pub const DEFAULT_SCRIPT_SIG: &str = "";
pub const DEFAULT_AMOUNT_0: u64 = 100000;
pub const DEFAULT_AMOUNT_0_ACCEPT_PEGIN: u64 = 99365;
pub const DEFAULT_AMOUNT_1: u64 = 0;
pub const DEFAULT_AMOUNT_1_ACCEPT_PEGIN: u64 = 300;
pub const DEFAULT_AMOUNT_IN_WEI: u64 = 1000000000000000;
pub const DEFAULT_V_OUT_ACCEPT_PEGIN: u32 = 0;

lazy_static::lazy_static! {
    pub static ref CONTRACTS_PATH: String = env::var("CONTRACTS_PATH")
        .unwrap_or_else(|_| "../../bitvmx-union-bridge-contracts".to_string());

    pub static ref TX_DISPATCHER_URL: String = env::var("TX_DISPATCHER_URL")
        .unwrap_or_else(|_| "http://localhost:3000".to_string());

    pub static ref ANVIL_URL: String = env::var("ANVIL_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8545".to_string());

    pub static ref KEY_STORE_PASSWORD: String = env::var("KEY_STORE_PASSWORD")
        .unwrap_or_else(|_| "p09ol.".to_string());

    pub static ref TRANSACTION_DISPATCHER_TOML_PATH: String = env::var("TRANSACTION_DISPATCHER_TOML_PATH")
        .unwrap_or_else(|_| "../transaction-dispatcher/Cargo.toml".to_string());
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

pub async fn call_endpoint(
    params: &HashMap<String, String>,
    endpoint: String,
    world: &mut TestWorld,
) -> (StatusCode, String) {
    let client = Client::new();
    let url = format!("{}{}", TX_DISPATCHER_URL.as_str(), endpoint);
    let payload = match endpoint.as_str() {
        "/pegin-address" => generate_payload_pegin_address(params),
        "/accept-pegin" => generate_payload_accept_pegin(params, world),
        "/register-pegin" => generate_payload_register_pegin(params, world),
        "/register-pegout" => generate_payload_register_pegout(params),
        _ => panic!("Unknown endpoint: {}", endpoint),
    };

    tracing::info!("POSTing to {} with payload: {}", url, payload);
    post_json(client, url, payload).await
}

pub fn extract_addresses(world: &TestWorld) -> (String, String) {
    let response_text_1 = world.response.as_ref().expect("No response received");
    let json_1: Value = serde_json::from_str(response_text_1).expect("response was not valid JSON");
    let address1 = json_1["address"]
        .as_str()
        .expect("response JSON has no string `address` field");
    let response_text_2 = world.response_2.as_ref().expect("No response 2 received");
    let json_2: Value =
        serde_json::from_str(response_text_2).expect("response 2 was not valid JSON");
    let address2 = json_2["address"]
        .as_str()
        .expect("response 2 JSON has no string `address` field");
    (address1.to_string(), address2.to_string())
}

fn generate_payload_pegin_address(params: &HashMap<String, String>) -> Value {
    serde_json::json!({
        "rootstock_deposit_address": params.get("rootstock_deposit_address").unwrap(),
        "value": params.get("value").unwrap().parse::<u64>().unwrap(),
        "btc_reimbursement_pub_key": params.get("btc_reimbursement_pub_key").unwrap()
    })
}

fn generate_payload_register_pegin(
    params: &HashMap<String, String>,
    world: &mut TestWorld,
) -> Value {
    let block_hash = get_or_generate("block_hash", params, world, &|| generate_random_hex(32));
    let merkle_branch_path = get_or_generate("merkle_branch_path", params, world, &|| {
        format!("0x{:08x}", SEEDED_RNG.lock().unwrap().random::<u32>())
    });
    let merkle_hash = get_or_generate("merkle_hash", params, world, &|| generate_random_hex(32));
    let tx_id = get_or_generate("tx_id", params, world, &|| generate_random_hex(32));

    let sequence = get_or_default("sequence", params, DEFAULT_SEQUENCE);
    let v_out = get_or_default("v_out", params, DEFAULT_V_OUT);
    let amount_0 = get_or_default("amount", params, DEFAULT_AMOUNT_0);
    let amount_1 = get_or_default("amount", params, DEFAULT_AMOUNT_1);

    let script_pub_key_0 = get_or_default(
        "script_pub_key_0",
        params,
        DEFAULT_SCRIPT_PUB_KEY_0.to_string(),
    );
    let script_pub_key_1 = get_or_default(
        "script_pub_key_1",
        params,
        DEFAULT_SCRIPT_PUB_KEY_1.to_string(),
    );

    let btc_tx = BtcTransaction {
        version: 2,
        outputs: vec![
            BtcOutput {
                amount: amount_0,
                script_pub_key: script_pub_key_0.clone(),
            },
            BtcOutput {
                amount: amount_1,
                script_pub_key: script_pub_key_1.clone(),
            },
        ],
        inputs: vec![BtcInput {
            tx_id: tx_id.clone(),
            v_out,
            sequence,
            script_sig: DEFAULT_SCRIPT_SIG.to_string(),
        }],
        lock_time: 0,
    };

    let btc_txid = compute_btc_txid(&btc_tx);

    world.register_pegin_block_hash = Some(block_hash.clone());
    world.register_pegin_btc_txid = Some(btc_txid.clone());
    world.register_pegin_merkle_hash = Some(merkle_hash.clone());

    serde_json::json!({
        "block_hash": block_hash,
        "btc_tx": btc_tx,
        "merkle_branch_hashes": [merkle_hash],
        "merkle_branch_path": merkle_branch_path
    })
}

fn generate_payload_accept_pegin(params: &HashMap<String, String>, world: &TestWorld) -> Value {
    let block_hash = world
        .register_pegin_block_hash
        .as_ref()
        .expect("Block hash must be present in world state")
        .clone();
    let merkle_hash = world
        .register_pegin_merkle_hash
        .as_ref()
        .expect("Merkle hash must be present in world state")
        .clone();
    let merkle_branch_path = get_or_generate("merkle_branch_path", params, world, &|| {
        format!("0x{:08x}", SEEDED_RNG.lock().unwrap().random::<u32>())
    });
    let tx_id = world
        .register_pegin_btc_txid
        .as_ref()
        .expect("Bitcoin tx id must be present in world state")
        .clone();

    let sequence = get_or_default("sequence", params, DEFAULT_SEQUENCE);
    let v_out = get_or_default("v_out", params, DEFAULT_V_OUT_ACCEPT_PEGIN);
    let amount_0 = get_or_default("amount", params, DEFAULT_AMOUNT_0_ACCEPT_PEGIN);
    let amount_1 = get_or_default("amount", params, DEFAULT_AMOUNT_1_ACCEPT_PEGIN);

    let script_pub_key_0 = get_or_default(
        "script_pub_key_0",
        params,
        DEFAULT_SCRIPT_PUB_KEY_0_ACCEPT_PEGIN.to_string(),
    );
    let script_pub_key_1 = get_or_default(
        "script_pub_key_1",
        params,
        DEFAULT_SCRIPT_PUB_KEY_1_ACCEPT_PEGIN.to_string(),
    );

    let btc_tx = BtcTransaction {
        version: 2,
        outputs: vec![
            BtcOutput {
                amount: amount_0,
                script_pub_key: script_pub_key_0.clone(),
            },
            BtcOutput {
                amount: amount_1,
                script_pub_key: script_pub_key_1.clone(),
            },
        ],
        inputs: vec![BtcInput {
            tx_id: tx_id.clone(),
            v_out,
            sequence,
            script_sig: DEFAULT_SCRIPT_SIG.to_string(),
        }],
        lock_time: 0,
    };

    serde_json::json!({
        "block_hash": block_hash,
        "btc_tx": btc_tx,
        "merkle_branch_hashes": [merkle_hash],
        "merkle_branch_path": merkle_branch_path
    })
}

fn generate_payload_register_pegout(params: &HashMap<String, String>) -> Value {
    let usr_pub_key = get_or_generate(
        "usr_pub_key",
        params,
        &TestWorld::default(),
        &generate_random_public_key,
    );
    let amount_in_wei = get_or_default("amount_in_wei", params, DEFAULT_AMOUNT_IN_WEI);
    serde_json::json!({
        "usr_pub_key": usr_pub_key,
        "amount_in_wei": amount_in_wei,
        "batch_flag": get_or_default("batch_flag", params, false)
    })
}

async fn post_json(client: Client, url: String, payload: Value) -> (StatusCode, String) {
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .expect("Failed to send POST request");

    let status = response.status();
    let response_text = response.text().await.expect("Failed to read response text");
    tracing::info!(
        "Response from {}:\nStatus: {}\nMessage: {}",
        url,
        status,
        response_text
    );
    (status, response_text)
}

fn get_or_generate(
    key: &str,
    params: &HashMap<String, String>,
    world: &TestWorld,
    default: &dyn Fn() -> String,
) -> String {
    params
        .get(key)
        .cloned()
        .or_else(|| get_from_world(key, world))
        .unwrap_or_else(|| default())
}

fn get_or_default<T: Clone + std::str::FromStr>(
    key: &str,
    params: &HashMap<String, String>,
    default: T,
) -> T
where
    T::Err: std::fmt::Debug,
{
    params
        .get(key)
        .and_then(|s| s.parse::<T>().ok())
        .unwrap_or(default)
}

fn compute_btc_txid(tx: &BtcTransaction) -> String {
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(tx.version).unwrap();
    write_varint(tx.inputs.len() as u64, &mut buf);

    tx.inputs.iter().for_each(|input| {
        let mut prev_tx_bytes =
            hex::decode(input.tx_id.trim_start_matches("0x")).expect("invalid hex in tx_id");
        prev_tx_bytes.reverse();
        buf.extend_from_slice(&prev_tx_bytes);
        buf.write_u32::<LittleEndian>(input.v_out).unwrap();

        let script_sig_bytes = hex::decode(input.script_sig.trim_start_matches("0x"))
            .expect("invalid hex in script_sig");
        write_varint(script_sig_bytes.len() as u64, &mut buf);
        buf.extend_from_slice(&script_sig_bytes);

        buf.write_u32::<LittleEndian>(input.sequence).unwrap();
    });

    write_varint(tx.outputs.len() as u64, &mut buf);
    tx.outputs.iter().for_each(|output| {
        buf.write_u64::<LittleEndian>(output.amount).unwrap();
        let spk_bytes = hex::decode(output.script_pub_key.trim_start_matches("0x"))
            .expect("invalid hex in script_pub_key");
        write_varint(spk_bytes.len() as u64, &mut buf);
        buf.extend_from_slice(&spk_bytes);
    });

    buf.write_u32::<LittleEndian>(tx.lock_time).unwrap();
    let first_hash = Sha256::digest(&buf);
    let second_hash = Sha256::digest(&first_hash);
    let mut txid_bytes = second_hash.to_vec();
    txid_bytes.reverse();
    format!("0x{}", hex::encode(txid_bytes))
}

fn write_varint(n: u64, writer: &mut Vec<u8>) {
    match n {
        n if n < 0xFD => writer.push(n as u8),
        n if n <= 0xFFFF => {
            writer.push(0xFD);
            writer.write_u16::<LittleEndian>(n as u16).unwrap();
        }
        n if n <= 0xFFFF_FFFF => {
            writer.push(0xFE);
            writer.write_u32::<LittleEndian>(n as u32).unwrap();
        }
        n => {
            writer.push(0xFF);
            writer.write_u64::<LittleEndian>(n).unwrap();
        }
    }
}

fn generate_random_hex(len: usize) -> String {
    let mut rng = SEEDED_RNG.lock().unwrap();
    let bytes: Vec<u8> = (0..len).map(|_| rng.random()).collect();
    format!("0x{}", hex::encode(bytes))
}

fn generate_random_public_key() -> String {
    let secp = Secp256k1::new();
    let mut rng = SEEDED_RNG.lock().unwrap();
    let mut sk_bytes = [0u8; 32];
    rng.fill_bytes(&mut sk_bytes);
    let sk = SecretKey::from_slice(&sk_bytes).expect("32 bytes, within curve order");
    let pk = PublicKey::from_secret_key(&secp, &sk);
    let compressed_pk = pk.serialize();
    format!("0x{}", hex::encode(compressed_pk))
}

fn get_from_world(key: &str, world: &TestWorld) -> Option<String> {
    match key {
        "tx_id" => world.register_pegin_btc_txid.clone(),
        "block_hash" => world.register_pegin_block_hash.clone(),
        "merkle_hash" => world.register_pegin_merkle_hash.clone(),
        _ => None,
    }
}

pub async fn wait_for_anvil() -> Result<(), String> {
    let client = reqwest::Client::new();
    let max_retries = 3;

    for _retry in 0..max_retries {
        if client.get(ANVIL_URL.as_str()).send().await.is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(format!(
        "Anvil failed to start after {} attempts",
        max_retries
    ))
}

pub async fn wait_for_transaction_dispatcher() -> Result<(), String> {
    let client = reqwest::Client::new();
    let max_retries = 3;

    for retry in 0..max_retries {
        if client
            .get(format!("{}/health", TX_DISPATCHER_URL.as_str()))
            .send()
            .await
            .is_ok()
        {
            return Ok(());
        }
        println!(
            "Waiting for transaction dispatcher to start... (attempt {}/{})",
            retry + 1,
            max_retries
        );
        thread::sleep(Duration::from_secs(1));
    }
    Err(format!(
        "Transaction dispatcher failed to start after {} attempts",
        max_retries
    ))
}

pub fn execute_script(script_path: &str) -> Result<String, String> {
    let full_path = get_contracts_path().join(script_path);
    execute_command(&format!("chmod +x {}", full_path.display()), false)?;
    execute_command(&format!("{}", full_path.display()), false)
}

pub fn execute_command(command: &str, spawn: bool) -> Result<String, String> {
    if spawn {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .spawn()
            .map_err(|e| format!("Failed to spawn command: {}", e))?;

        thread::sleep(Duration::from_secs(1));

        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                Err(format!("Command failed with status: {}", status))
            }
            Ok(None) => Ok("Command spawned successfully".to_string()),
            Ok(Some(status)) => Ok(format!("Command completed with status: {}", status)),
            Err(e) => Err(format!("Failed to check command status: {}", e)),
        }
    } else {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

pub fn get_contracts_path() -> PathBuf {
    std::env::current_dir()
        .expect("Failed to get current directory")
        .join(CONTRACTS_PATH.as_str())
}

pub async fn start_anvil() -> Result<String, String> {
    execute_command("anvil", true)?;
    wait_for_anvil().await?;
    Ok("Anvil started successfully".to_string())
}

pub fn transfer_ether(from: &str, to: &str, amount: &str) -> Result<String, String> {
    let command = format!(
        "cast send --rpc-url {} --from {} {} --value {} --unlocked",
        ANVIL_URL.as_str(),
        from,
        to,
        amount
    );
    execute_command(&command, false)
}

pub async fn start_transaction_dispatcher() -> Result<String, String> {
    let command = format!(
        "KEY_STORE_PASSWORD={} cargo run --manifest-path {} --bin transaction-dispatcher",
        KEY_STORE_PASSWORD.as_str(),
        TRANSACTION_DISPATCHER_TOML_PATH.as_str()
    );
    execute_command(&command, true)?;
    wait_for_transaction_dispatcher().await?;
    Ok("Transaction dispatcher started successfully".to_string())
}
