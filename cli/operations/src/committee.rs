use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use op_funding::derive_stream_funding_profile;
use protocol_params::{committee_member_count, prover_count, slots_per_package};
use reqwest::Client;
use serde::Serialize;
use tokio::time::sleep;

use crate::environments::Environment;
use crate::utils::{confirm_operation, request_to_string};
use crate::validate_1_10;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub(crate) enum CommitteeRole {
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
            _ => Err(format!("Invalid role: {}. Expected Prover or Verifier", input)),
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
    advance_funds: Funding,
}

#[derive(Debug, Serialize)]
struct Funding {
    value: u64,
}

pub(crate) async fn run_committee_setup(
    stream_id: u64,
    environment: Environment,
    operator_id: Option<u8>,
    role: Option<CommitteeRole>,
) -> Result<()> {
    let client = Client::new();

    let endpoints = environment.user_api_endpoints()?;

    match environment {
        Environment::LocalAnvil
        | Environment::DockerAnvil
        | Environment::LocalRskj
        | Environment::DockerRskj => {
            if role.is_some() {
                eprintln!("Warning: --role is ignored in local environment");
            }

            for (idx, endpoint) in endpoints.iter().enumerate() {
                let role =
                    if idx % 2 == 0 { CommitteeRole::Prover } else { CommitteeRole::Verifier };

                post_apply(&client, stream_id, endpoint, role, &environment).await?;

                if idx + 1 != endpoints.len() {
                    sleep(Duration::from_secs(2)).await;
                }
            }

            println!(
                "Done. Applied {} operators to stream {} (2 Provers, 2 Verifiers)",
                endpoints.len(),
                stream_id
            );
        }
        Environment::Remote(_) => {
            let role = role.ok_or_else(|| {
                anyhow!("--role is required when using --env {}", environment.get_name())
            })?;

            let op_id = operator_id.ok_or_else(|| {
                anyhow!("--operator-id is required when using --env {}", environment.get_name())
            })?;

            validate_1_10(op_id, "operator-id")?;

            let endpoint = endpoints
                .get((op_id - 1) as usize)
                .ok_or_else(|| anyhow!("Invalid operator-id {}", op_id))?;

            post_apply(&client, stream_id, endpoint, role, &environment).await?;

            println!("Done. Applied operator {} to stream {} as {}", op_id, stream_id, role);
        }
    }

    Ok(())
}

async fn post_apply(
    client: &Client,
    stream_id: u64,
    endpoint: &str,
    role: CommitteeRole,
    environment: &Environment,
) -> Result<()> {
    let funding_profile = derive_stream_funding_profile(
        stream_id,
        environment.uses_bitcoin_regtest(),
        slots_per_package()?,
        committee_member_count()?,
        prover_count()?,
    )?;

    let payload = ApplyStreamRequest {
        apply_to_stream: ApplyToStream {
            stream_id,
            role: role.as_str().to_string(),
            funding_utxo: Funding { value: funding_profile.protocol_funding },
            speed_up_utxo: Funding { value: funding_profile.speed_up_utxo },
            advance_funds: Funding { value: funding_profile.advance_funds },
        },
    };

    let url = format!("http://{}/member/apply-stream", endpoint);

    let request = client.post(&url).json(&payload).build()?;

    if environment.is_remote() {
        let description = request_to_string(&request);
        if !confirm_operation(&description)? {
            bail!("Operation cancelled by user");
        }
    } else {
        println!("Applying operator on {} as {}...", endpoint, role);
    }

    let response = client
        .execute(request)
        .await
        .with_context(|| format!("Failed to connect to operator at {}", endpoint))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!("Operator at {} responded with status {}: {}", endpoint, status, body);
    }

    Ok(())
}
