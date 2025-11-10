use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const USER_API_PEGIN_ENDPOINT: &str = "http://localhost:40001/user/pegin-address";

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

pub async fn create_pegin_tx(
    rsk_address: String,
    stream_amount: u64,
    packet_number: u64,
) -> Result<()> {
    validate_rsk_address(&rsk_address)?;
    println!("Getting pegin address from user-api...");

    let request = PeginAddressRequest {
        rootstock_deposit_address: rsk_address.clone(),
        value: stream_amount,
        btc_reimbursement_pub_key: String::new(),
    };

    let client = Client::new();
    let response = client
        .post(USER_API_PEGIN_ENDPOINT)
        .json(&request)
        .send()
        .await
        .context("Failed to connect to user-api")?;

    let status = response.status();
    if !status.is_success() {
        let body = match response.text().await {
            Ok(body) => body,
            Err(_) => String::from("<failed to read response body>"),
        };
        bail!("user-api responded with status {}: {}", status, body);
    }

    let pegin_response: PeginAddressResponse = response
        .json()
        .await
        .context("Failed to parse user-api response")?;

    let pegin_address = pegin_response
        .address
        .filter(|addr| !addr.is_empty())
        .ok_or_else(|| anyhow!("user-api response did not contain a pegin address"))?;

    println!("Parameters:");
    println!("  Stream amount: {}", stream_amount);
    println!("  Packet number: {}", packet_number);
    println!("  RSK address: {}", rsk_address);
    println!("  Pegin address: {}", pegin_address);
    println!();
    println!("Now run the following command in bitcoin-wallet CLI (user mode):");
    println!();
    println!(
        "create_pegin_tx {} {} {} {}",
        stream_amount, packet_number, pegin_address, rsk_address
    );

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
