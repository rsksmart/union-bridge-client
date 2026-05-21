use anyhow::{Context, Result, anyhow, bail};
use bitcoin::PrivateKey;
use bitcoin::secp256k1::Secp256k1;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::environments::Environment;
use crate::utils::{confirm_operation, run_wallet_command};

#[derive(Debug, Serialize)]
struct PeginAddressRequest {
    rootstock_deposit_address: String,
    value: u64,
    btc_reimbursement_pub_key: String,
}

#[derive(Debug, Deserialize)]
struct PeginAddressResponse {
    address: Option<String>,
    packet_number: Option<u64>,
    enabler_script_pubkey: Option<String>,
}

pub(crate) async fn create_pegin_tx(
    environment: Environment,
    rsk_address: String,
    value: u64,
    execute: bool,
) -> Result<()> {
    if execute && environment.is_remote() {
        bail!(
            "--execute flag is only supported for local environments (`local`/`docker`). For remote environments, please run the wallet commands manually."
        );
    }

    validate_rsk_address(&rsk_address)?;

    let env_name = environment.get_name();
    println!("Environment: {}", env_name);
    println!();

    if environment.is_remote() {
        let description = format!(
            "Pegin summary:\n  RSK address: {}\n  Value:       {} sats",
            rsk_address, value
        );
        if !confirm_operation(&description)? {
            println!("Aborted.");
            return Ok(());
        }
        println!();
    }

    println!("Getting pegin data for {rsk_address}...");

    let btc_reimbursement_pub_key = derive_reimbursement_xonly_pub_key()?;
    let payload = PeginAddressRequest {
        rootstock_deposit_address: rsk_address.clone(),
        value,
        btc_reimbursement_pub_key,
    };

    let user_api_base = environment
        .user_api_endpoints()?
        .first()
        .expect("No local user-api endpoints configured; please review your config")
        .to_string();

    let endpoint = format!("http://{}/user/pegin-address", user_api_base);

    let client = Client::new();
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

    let packet_number = pegin_response
        .packet_number
        .ok_or_else(|| anyhow!("user-api response did not contain a packet_number"))?;

    let enabler_script_pubkey = pegin_response
        .enabler_script_pubkey
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("user-api response did not contain enabler_script_pubkey"))?;

    println!("Requesting pegin: {} sats", value);
    println!("  Destination: RSK {}", rsk_address);
    println!();
    println!("Parameters:");
    println!("  Value: {}", value);
    println!("  Packet number: {}", packet_number);
    println!("  Pegin address: {}", pegin_address);
    println!("  Enabler script: {}", enabler_script_pubkey);
    println!();

    if execute {
        println!("Executing wallet command programmatically...");
        println!();
        let stdout = run_wallet_command(&[
            "user",
            "create_pegin_tx",
            &value.to_string(),
            &packet_number.to_string(),
            &pegin_address,
            &rsk_address,
            &enabler_script_pubkey,
        ])?;
        println!("{}", stdout);
    } else {
        println!("Now run the following command in bitcoin-wallet CLI (user mode):");
        println!();
        println!(
            "create_pegin_tx {} {} {} {} {}",
            value, packet_number, pegin_address, rsk_address, enabler_script_pubkey
        );
    }

    Ok(())
}

fn derive_reimbursement_xonly_pub_key() -> Result<String> {
    let wif = std::env::var("USER_BITCOIN_WIF")
        .context("USER_BITCOIN_WIF environment variable not set")?;
    let private_key =
        PrivateKey::from_wif(&wif).context("failed to parse USER_BITCOIN_WIF as WIF")?;
    let public_key = private_key.public_key(&Secp256k1::new());
    let (xonly, _) = public_key.inner.x_only_public_key();
    Ok(format!("0x{}", hex::encode(xonly.serialize())))
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
