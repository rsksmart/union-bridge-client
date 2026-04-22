use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::environments::Environment;
use crate::rsk_wallet::get_user_rsk_address;
use crate::utils::{confirm_operation, request_to_string};

#[derive(Debug, Serialize)]
struct RequestPegoutPayload {
    amount_in_wei: u64,
    usr_pub_key: String,
}

#[derive(Debug, Deserialize)]
struct PegoutResponse {
    result: Option<String>,
    error: Option<String>,
}

/// converts satoshis to wei
/// 1 sat = 10^10 wei (since 1 BTC = 10^8 sats and 1 BTC = 10^18 wei)
fn sats_to_wei(sats: u64) -> u64 {
    sats.saturating_mul(10_000_000_000)
}

pub async fn request_pegout(
    environment: Environment,
    value: u64,
    usr_pub_key: String,
) -> Result<()> {
    validate_usr_pub_key(&usr_pub_key)?;
    let amount_in_wei = sats_to_wei(value);

    let rsk_address = get_user_rsk_address(&environment, true)
        .await?
        .unwrap_or_else(|| "<unknown - check user-api /user/rsk-address>".to_string());

    println!("Requesting pegout: {} sats ({} wei)", value, amount_in_wei);
    println!("  Source:      RSK {}", rsk_address);
    println!("  Destination: Bitcoin (public key: {})", usr_pub_key);

    let user_api_base = environment
        .user_api_endpoints()?
        .first()
        .expect("No user-api endpoints configured; please review your config")
        .to_string();

    let endpoint = format!("http://{}/user/request-pegout", user_api_base);

    let payload = RequestPegoutPayload { amount_in_wei, usr_pub_key };

    let client = Client::new();
    let request = client.post(&endpoint).json(&payload).build()?;

    if environment.is_remote() {
        let description = request_to_string(&request);
        if !confirm_operation(&description)? {
            bail!("Operation cancelled by user");
        }
    }

    let response = client
        .execute(request)
        .await
        .with_context(|| format!("Failed to connect to user-api at {}", endpoint))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!("user-api responded with status {}: {}", status, body);
    }

    let pegout_response: PegoutResponse =
        response.json().await.context("Failed to parse user-api response")?;

    if let Some(error) = pegout_response.error {
        bail!("Pegout request failed: {}", error);
    }

    println!("Pegout request successful!");
    if let Some(result) = pegout_response.result {
        println!("Result: {}", result);
    }

    Ok(())
}

fn validate_usr_pub_key(key: &str) -> Result<()> {
    let stripped = key
        .strip_prefix("0x")
        .or_else(|| key.strip_prefix("0X"))
        .ok_or_else(|| anyhow::anyhow!("User public key must start with 0x"))?;

    if stripped.len() != 66 {
        bail!("User public key must be a 33-byte compressed pubkey (66 hex characters after 0x prefix)");
    }

    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("User public key must contain only hexadecimal characters");
    }

    Ok(())
}
