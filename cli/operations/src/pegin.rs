use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::environments::Environment;

#[derive(Debug, Serialize)]
struct PeginAddressRequest {
    rootstock_deposit_address: String,
    value: u64,
    btc_reimbursement_pub_key: String,
}

#[derive(Debug, Deserialize)]
struct PeginAddressResponse {
    address: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: &'static str,
    params: (EthCallParams, &'static str),
    id: u64,
}

#[derive(Debug, Serialize)]
struct EthCallParams {
    to: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<String>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    message: String,
}

/// Fetches the current pegin packet number from the StreamManager contract.
///
/// Calls `getStreamById(uint64)` (selector `0xf0c3a028`) and extracts the
/// `peginPacketPointer` field (3rd uint64 in the returned Stream struct).
async fn fetch_packet_number(
    client: &Client,
    rpc_url: &str,
    stream_manager_address: &str,
    stream_id: u64,
) -> Result<u64> {
    // ABI-encode: selector (4 bytes) + stream_id as uint64 padded to 32 bytes
    let data = format!("0xf0c3a028{:064x}", stream_id);

    let rpc_request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_call",
        params: (EthCallParams { to: stream_manager_address.to_string(), data }, "latest"),
        id: 1,
    };

    let response = client
        .post(rpc_url)
        .json(&rpc_request)
        .send()
        .await
        .context("Failed to connect to RPC endpoint for packet number query")?;

    let rpc_response: JsonRpcResponse =
        response.json().await.context("Failed to parse RPC response for packet number query")?;

    if let Some(err) = rpc_response.error {
        bail!("RPC error querying packet number: {}", err.message);
    }

    let result_hex = rpc_response
        .result
        .ok_or_else(|| anyhow!("RPC response missing result for packet number query"))?;

    // The result is ABI-encoded Stream struct:
    //   streamId (32 bytes) | denomination (32 bytes) | peginPacketPointer (32 bytes) | ...
    // peginPacketPointer starts at byte offset 64 (hex chars 128+4 for "0x" prefix)
    let hex = result_hex.strip_prefix("0x").unwrap_or(&result_hex);

    // Each field is 32 bytes = 64 hex chars. peginPacketPointer is the 3rd field.
    let field_start = 2 * 64; // skip streamId and denomination
    let field_end = field_start + 64;

    if hex.len() < field_end {
        bail!(
            "RPC response too short to contain peginPacketPointer (got {} hex chars, need {})",
            hex.len(),
            field_end
        );
    }

    let pointer_hex = &hex[field_start..field_end];
    let packet_number = u64::from_str_radix(pointer_hex.trim_start_matches('0'), 16).unwrap_or(0);

    Ok(packet_number)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_pegin_tx(
    environment: Environment,
    rsk_address: String,
    value: u64,
    stream_id: u64,
    packet_number: Option<u64>,
    stream_manager_address: Option<String>,
    btc_pub_key: String,
    execute: bool,
) -> Result<()> {
    if execute && environment.is_remote() {
        bail!("--execute flag is only supported for local environments (local/local-docker). For remote environments, please run the wallet commands manually.");
    }

    validate_rsk_address(&rsk_address)?;
    validate_btc_pub_key(&btc_pub_key)?;

    let client = Client::new();

    // Resolve StreamManager address: use provided override or environment default
    let env_default = environment.stream_manager_address();
    let sm_address = stream_manager_address.as_deref().unwrap_or(&env_default);

    // Resolve packet number: use provided value or auto-fetch from StreamManager
    let packet_number = match packet_number {
        Some(n) => {
            println!("Using provided packet number: {}", n);
            n
        }
        None => {
            println!(
                "Fetching packet number from StreamManager {} (stream_id={})...",
                sm_address, stream_id
            );
            let n =
                fetch_packet_number(&client, &environment.rpc_url(), sm_address, stream_id).await?;
            println!("Auto-calculated packet number: {}", n);
            n
        }
    };

    println!("Getting pegin address for {rsk_address}...");

    let payload = PeginAddressRequest {
        rootstock_deposit_address: rsk_address.clone(),
        value,
        btc_reimbursement_pub_key: btc_pub_key.clone(),
    };

    let user_api_base = environment
        .user_api_endpoints()
        .first()
        .expect("No local user-api endpoints configured; please review your config")
        .to_string();

    let endpoint = format!("http://{}/user/pegin-address", user_api_base);

    let request = client.post(&endpoint).json(&payload).build()?;

    let response = client.execute(request).await.context("Failed to connect to user-api")?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!("user-api responded with status {}: {}", status, body);
    }

    let pegin_response: PeginAddressResponse =
        response.json().await.context("Failed to parse user-api response")?;

    let pegin_address = pegin_response
        .address
        .filter(|addr| !addr.is_empty())
        .ok_or_else(|| anyhow!("user-api response did not contain a pegin address"))?;

    println!("Requesting pegin: {} sats", value);
    println!("  Source:      Bitcoin (public key: {})", btc_pub_key);
    println!("  Destination: RSK {}", rsk_address);
    println!();
    println!("Parameters:");
    println!("  Value: {}", value);
    println!("  Packet number: {}", packet_number);
    println!("  Pegin address: {}", pegin_address);
    println!();

    if execute {
        println!("Executing wallet command programmatically...");
        println!();
        execute_wallet_command(value, packet_number, &pegin_address, &rsk_address)?;
    } else {
        println!("Now run the following command in bitcoin-wallet CLI (user mode):");
        println!();
        println!("create_pegin_tx {} {} {} {}", value, packet_number, pegin_address, rsk_address);
    }

    Ok(())
}

fn execute_wallet_command(
    stream_amount: u64,
    packet_number: u64,
    pegin_address: &str,
    rsk_address: &str,
) -> Result<()> {
    let wallet_script = "./cli-bitcoin-wallet.sh";

    let mut cmd = Command::new(wallet_script);
    cmd.arg("user")
        .arg("create_pegin_tx")
        .arg(stream_amount.to_string())
        .arg(packet_number.to_string())
        .arg(pegin_address)
        .arg(rsk_address);

    println!(
        "Running: {} user create_pegin_tx {} {} {} {}",
        wallet_script, stream_amount, packet_number, pegin_address, rsk_address
    );

    let output = cmd.output().context("failed to execute cli-bitcoin-wallet.sh")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "wallet command failed with status {}:\nstdout: {}\nstderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);

    Ok(())
}

fn validate_rsk_address(address: &str) -> Result<()> {
    let stripped = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .ok_or_else(|| anyhow!("RSK address must start with 0x"))?;

    if stripped.len() != 40 {
        bail!("RSK address must have 40 hex characters after the 0x prefix");
    }

    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("RSK address must contain only hexadecimal characters");
    }

    Ok(())
}

fn validate_btc_pub_key(key: &str) -> Result<()> {
    let stripped = key
        .strip_prefix("0x")
        .or_else(|| key.strip_prefix("0X"))
        .ok_or_else(|| anyhow!("BTC public key must start with 0x"))?;

    if stripped.len() != 64 {
        bail!("BTC public key must be a 32-byte x-only pubkey (64 hex characters after 0x prefix)");
    }

    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("BTC public key must contain only hexadecimal characters");
    }

    Ok(())
}
