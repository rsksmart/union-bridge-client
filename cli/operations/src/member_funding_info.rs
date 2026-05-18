use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;

use crate::environments::Environment;

#[derive(Debug, Clone, Deserialize)]
pub struct MemberFundingInfo {
    pub bitcoin_address: String,
    pub rsk_address: String,
}

pub type CollectedMemberFundingInfo = Vec<(String, MemberFundingInfo)>;

pub async fn collect_member_funding_info(
    env: &Environment,
    first_only: bool,
) -> Result<CollectedMemberFundingInfo> {
    let mut endpoints = env.user_api_endpoints()?;
    if first_only {
        endpoints.truncate(1);
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build http client")?;

    let mut results = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let url = format!("http://{}/member/funding-info", endpoint);
        let response =
            client.get(&url).send().await.with_context(|| format!("failed to fetch {}", url))?;

        if !response.status().is_success() {
            bail!("request to {} failed with status {}", url, response.status());
        }

        let body: MemberFundingInfo = response
            .json()
            .await
            .with_context(|| format!("failed to decode response body from {}", url))?;
        results.push((endpoint, body));
    }

    Ok(results)
}
