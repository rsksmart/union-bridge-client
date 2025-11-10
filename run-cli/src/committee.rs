use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use reqwest::Client;
use serde::Serialize;
use tokio::time::sleep;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CommitteeEnv {
    Local,
    Alphanet,
}

impl Default for CommitteeEnv {
    fn default() -> Self {
        CommitteeEnv::Local
    }
}

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
    environment: CommitteeEnv,
    role: Option<CommitteeRole>,
) -> Result<()> {
    let client = Client::new();

    match environment {
        CommitteeEnv::Local => {
            if role.is_some() {
                eprintln!("Warning: --role is ignored in local environment");
            }

            let steps = [
                (40001u16, CommitteeRole::Prover),
                (40002u16, CommitteeRole::Prover),
                (40003u16, CommitteeRole::Verifier),
                (40004u16, CommitteeRole::Verifier),
            ];

            for (idx, (port, role)) in steps.iter().enumerate() {
                post_apply(&client, stream_id, *port, *role).await?;

                if idx + 1 != steps.len() {
                    sleep(Duration::from_secs(5)).await;
                }
            }

            println!(
                "Done. Applied 4 operators to stream {} (2 Provers, 2 Verifiers)",
                stream_id
            );
        }
        CommitteeEnv::Alphanet => {
            let role =
                role.ok_or_else(|| anyhow!("--role is required when using --env alphanet"))?;

            post_apply(&client, stream_id, 40001, role).await?;

            println!("Done. Applied operator to stream {} as {}", stream_id, role);
        }
    }

    Ok(())
}

async fn post_apply(client: &Client, stream_id: u64, port: u16, role: CommitteeRole) -> Result<()> {
    let request = ApplyStreamRequest {
        apply_to_stream: ApplyToStream {
            stream_id,
            role: role.as_str().to_string(),
            funding_utxo: Funding { value: 10_000_000 },
            speed_up_utxo: Funding { value: 10_000_000 },
        },
    };

    println!("Applying operator on port {} as {}...", port, role);

    let response = client
        .post(format!("http://localhost:{}/member/apply-stream", port))
        .json(&request)
        .send()
        .await
        .with_context(|| format!("Failed to connect to operator on port {}", port))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!(
            "Operator on port {} responded with status {}: {}",
            port,
            status,
            body
        );
    }

    Ok(())
}
