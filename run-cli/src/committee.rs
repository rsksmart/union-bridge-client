use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use reqwest::Client;
use serde::Serialize;
use tokio::time::sleep;

use crate::environments::Environment;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CommitteeRole {
    Prover,
    Verifier,
}

impl CommitteeRole {
    fn as_str(&self) -> &'static str {
        match self {
            CommitteeRole::Prover => "Prover",
            CommitteeRole::Verifier => "Verifier",
        }
    }
}

impl fmt::Display for CommitteeRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CommitteeRole {
    type Err = String;

    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        match input.to_lowercase().as_str() {
            "prover" => Ok(CommitteeRole::Prover),
            "verifier" => Ok(CommitteeRole::Verifier),
            _ => Err(format!(
                "Invalid role: {}. Expected Prover or Verifier",
                input
            )),
        }
    }
}

#[derive(Debug, Serialize)]
struct ApplyStreamRequest {
    #[serde(rename = "ApplyToStream")]
    apply_to_stream: ApplyToStream,
}

#[derive(Debug, Serialize)]
struct ApplyToStream {
    stream_id: u64,
    role: String,
    funding_utxo: Funding,
    speed_up_utxo: Funding,
}

#[derive(Debug, Serialize)]
struct Funding {
    value: u64,
}

pub async fn run_committee_setup(
    stream_id: u64,
    environment: Environment,
    role: Option<CommitteeRole>,
) -> Result<()> {
    let client = Client::new();

    let endpoints = environment.user_api_endpoints();

    match environment {
        Environment::Local => {
            if role.is_some() {
                eprintln!("Warning: --role is ignored in local environment");
            }

            for (idx, endpoint) in endpoints.iter().enumerate() {
                let role = if idx % 2 == 0 {
                    CommitteeRole::Prover
                } else {
                    CommitteeRole::Verifier
                };

                post_apply(&client, stream_id, endpoint, role).await?;

                if idx + 1 != environment.user_api_endpoints().len() {
                    sleep(Duration::from_secs(5)).await;
                }
            }

            println!(
                "Done. Applied {} operators to stream {} (2 Provers, 2 Verifiers)",
                environment.user_api_endpoints().len(),
                stream_id
            );
        }
        Environment::Alphanet | Environment::Testnet => {
            let role = role.ok_or_else(|| {
                anyhow!(
                    "--role is required when using --env {}",
                    environment.get_name()
                )
            })?;

            let endpoint = endpoints
                .first()
                .expect("No user-api endpoints configured; please review your config");

            post_apply(&client, stream_id, endpoint, role).await?;

            println!("Done. Applied operator to stream {} as {}", stream_id, role);
        }
        Environment::LocalDocker => {
            bail!("Environment::LocalDocker is not supported for committee setup. Use Local, Alphanet, or Testnet.");
        }
    }

    Ok(())
}

async fn post_apply(
    client: &Client,
    stream_id: u64,
    endpoint: &str,
    role: CommitteeRole,
) -> Result<()> {
    let request = ApplyStreamRequest {
        apply_to_stream: ApplyToStream {
            stream_id,
            role: role.as_str().to_string(),
            funding_utxo: Funding { value: 10_000_000 },
            speed_up_utxo: Funding { value: 10_000_000 },
        },
    };

    println!("Applying operator on {} as {}...", endpoint, role);

    let url = format!("http://{}/member/apply-stream", endpoint);
    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .with_context(|| format!("Failed to connect to operator at {}", endpoint))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!(
            "Operator at {} responded with status {}: {}",
            endpoint,
            status,
            body
        );
    }

    Ok(())
}
