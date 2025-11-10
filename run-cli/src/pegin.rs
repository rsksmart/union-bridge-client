use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Args;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

#[derive(Debug, Args, Clone)]
pub struct CreatePeginTxArgs {
    /// Rootstock address that will receive the pegin funds
    #[arg(long = "rsk-address", env = "RSK_ADDRESS")]
    rsk_address: String,

    /// Base URL for the user-api service that returns pegin addresses
    #[arg(
        long = "user-api-url",
        env = "USER_API_URL",
        default_value = "http://localhost:40001"
    )]
    user_api_url: String,

    /// Amount of BTC to stream in satoshis
    #[arg(long = "stream-amount", default_value_t = 1_000_000)]
    stream_amount: u64,

    /// Packet number used when constructing the pegin transaction
    #[arg(long = "packet-number", default_value_t = 0)]
    packet_number: u32,
}

#[derive(Debug, Serialize)]
struct PeginAddressRequest {
    rootstock_deposit_address: String,
    value: u64,
    btc_reimbursement_pub_key: String,
}

#[derive(Debug, Deserialize)]
struct PeginAddressResponse {
    address: String,
}

pub async fn create_pegin_tx(args: CreatePeginTxArgs) -> Result<()> {
    let CreatePeginTxArgs {
        rsk_address,
        stream_amount,
        packet_number,
        user_api_url,
    } = args;

    println!("Getting pegin address from user-api at {}...", user_api_url);

    let client = PeginClient::new(&user_api_url)?;

    let request_body = PeginAddressRequest {
        rootstock_deposit_address: rsk_address.clone(),
        value: stream_amount,
        btc_reimbursement_pub_key: String::new(),
    };

    let pegin_address = client.fetch_pegin_address(&request_body).await?;

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

struct PeginClient {
    http: Client,
    pegin_endpoint: Url,
}

impl PeginClient {
    fn new(base_url: &str) -> Result<Self> {
        let mut pegin_endpoint = Url::parse(base_url)
            .with_context(|| format!("Invalid user-api URL: {}", base_url))?;
        pegin_endpoint.set_path("user/pegin-address");
        pegin_endpoint.set_query(None);

        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            http,
            pegin_endpoint,
        })
    }

    async fn fetch_pegin_address(&self, request: &PeginAddressRequest) -> Result<String> {
        let response = self
            .http
            .post(self.pegin_endpoint.clone())
            .json(request)
            .send()
            .await
            .context("Failed to request pegin address from user-api")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read response body from user-api")?;

        if !status.is_success() {
            bail!(
                "Failed to get pegin address: user-api returned status {}. Body: {}",
                status,
                body.trim()
            );
        }

        let pegin_address = serde_json::from_str::<PeginAddressResponse>(&body)
            .with_context(|| {
                format!(
                    "Failed to parse pegin address from response body: {}",
                    body.trim()
                )
            })?
            .address;

        Ok(pegin_address)
    }
}
