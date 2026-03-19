use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::environments::Environment;
use crate::utils::{confirm_operation, request_to_string};

#[derive(Debug, Serialize)]
struct RejectPeginPayload {
    committee_id: String,
    member_index: usize,
    request_pegin_txid: String,
}

#[derive(Debug, Deserialize)]
struct RejectPeginResponse {
    result: Option<String>,
    error: Option<String>,
}

pub async fn request_reject_pegin(
    environment: Environment,
    operator_id: Option<u8>,
    committee_id: String,
    member_index: usize,
    request_pegin_txid: String,
) -> Result<()> {
    validate_committee_id(&committee_id)?;
    let request_pegin_txid = normalize_txid(&request_pegin_txid)?;
    let endpoint = resolve_member_endpoint(environment, operator_id)?;
    let payload = RejectPeginPayload { committee_id, member_index, request_pegin_txid };
    let url = format!("http://{}/member/reject-pegin", endpoint);

    println!("Requesting reject pegin on {}...", endpoint);
    println!("  Committee ID: {}", payload.committee_id);
    println!("  Member index: {}", payload.member_index);
    println!("  Request pegin txid: {}", payload.request_pegin_txid);

    let client = Client::new();
    let request = client.post(&url).json(&payload).build()?;

    if environment.is_remote() {
        let description = request_to_string(&request);
        if !confirm_operation(&description)? {
            bail!("Operation cancelled by user");
        }
    }

    let response = client
        .execute(request)
        .await
        .with_context(|| format!("Failed to connect to member endpoint at {}", url))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<failed to read response body>"));
        bail!("member endpoint responded with status {}: {}", status, body);
    }

    let reject_response: RejectPeginResponse =
        response.json().await.context("Failed to parse member endpoint response")?;

    if let Some(error) = reject_response.error {
        bail!("Reject pegin request failed: {}", error);
    }

    println!("Reject pegin request successful!");
    if let Some(result) = reject_response.result {
        println!("Result: {}", result);
    }

    Ok(())
}

fn resolve_member_endpoint(environment: Environment, operator_id: Option<u8>) -> Result<String> {
    let endpoints = environment.user_api_endpoints();

    let selected_operator_id = match environment {
        Environment::Local | Environment::LocalDocker => operator_id.unwrap_or(1),
        Environment::Regtest | Environment::Alphanet | Environment::Testnet => operator_id
            .ok_or_else(|| {
                anyhow!("--operator-id is required when using --env {}", environment.get_name())
            })?,
    };

    if selected_operator_id == 0 {
        bail!("operator-id must be at least 1");
    }

    endpoints.get((selected_operator_id - 1) as usize).cloned().ok_or_else(|| {
        anyhow!("Invalid operator-id {} for --env {}", selected_operator_id, environment.get_name())
    })
}

fn validate_committee_id(committee_id: &str) -> Result<()> {
    if committee_id.is_empty() {
        bail!("committee-id cannot be empty");
    }

    if !committee_id.chars().all(|c| c.is_ascii_digit()) {
        bail!("committee-id must be a decimal string");
    }

    committee_id
        .parse::<u128>()
        .map_err(|_| anyhow!("committee-id must fit in an unsigned 128-bit integer"))?;

    Ok(())
}

fn normalize_txid(txid: &str) -> Result<String> {
    let stripped = txid.strip_prefix("0x").or_else(|| txid.strip_prefix("0X")).unwrap_or(txid);

    if stripped.len() != 64 {
        bail!("request-pegin-txid must be a 32-byte hex string (64 hex characters)");
    }

    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("request-pegin-txid must contain only hexadecimal characters");
    }

    Ok(format!("0x{}", stripped.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_txid_with_or_without_prefix() {
        let txid = "4E80F8119C7299AE9D85ADAD5F0A45BAA69831069046569EF4BA9574249EE471";

        assert_eq!(
            normalize_txid(txid).expect("txid without prefix should be valid"),
            "0x4e80f8119c7299ae9d85adad5f0a45baa69831069046569ef4ba9574249ee471"
        );
        assert_eq!(
            normalize_txid(&format!("0x{txid}")).expect("txid with prefix should be valid"),
            "0x4e80f8119c7299ae9d85adad5f0a45baa69831069046569ef4ba9574249ee471"
        );
    }

    #[test]
    fn rejects_non_decimal_committee_id() {
        let err = validate_committee_id("abc").expect_err("committee id must be decimal");
        assert!(err.to_string().contains("decimal string"));
    }

    #[test]
    fn local_defaults_to_operator_one() {
        let endpoint =
            resolve_member_endpoint(Environment::Local, None).expect("local should default");
        assert_eq!(endpoint, "localhost:40001");
    }

    #[test]
    fn remote_requires_operator_id() {
        let err = resolve_member_endpoint(Environment::Alphanet, None)
            .expect_err("remote must require operator id");
        assert!(err.to_string().contains("--operator-id is required"));
    }
}
