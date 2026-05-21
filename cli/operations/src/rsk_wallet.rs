use std::process::Command;
use std::str::FromStr;

use alloy_primitives::U256;
use anyhow::{Context, Result, anyhow, bail};
use op_funding::{derive_stream_funding_profile, required_member_rsk_balance};
use protocol_params::{committee_member_count, prover_count, slots_per_package};
use reqwest::Client;
use rpassword::prompt_password;
use serde::Deserialize;

use crate::bitcoin_wallet::derive_user_bitcoin_address_from_env;
use crate::constants::{LOCAL_ANVIL_ADDRESS, operator_ids};
use crate::environments::*;
use crate::member_funding_info::CollectedMemberFundingInfo;

// Keep this aligned with `union-bridge-client/config/base.toml`. Local and docker both point at
// the same Anvil deployment, so the CLI can rely on this fixed StreamManager address.
const LOCAL_STREAM_MANAGER_ADDRESS: &str = "0x0165878A594ca255338adfa4d48449f69242Eb8F";
const WEI_PER_RBTC: u64 = 1_000_000_000_000_000_000;
const WEI_PER_SAT: u64 = 10_000_000_000;
// Fixed local/dev gas headroom added on top of the pegout amount for user wallets.
const LOCAL_USER_RSK_GAS_BUFFER_WEI: u64 = 30_000_000_000_000_000;

#[derive(Deserialize)]
struct AddressResponse {
    address: String,
}

/// whitelists member RSK addresses on the CommitteeRegistry contract.
/// collects member signer addresses from staged keystores, then calls
/// `whitelistAddresses(address[])` via `cast send`.
pub(crate) async fn handle_whitelist(
    env: Environment,
    contract_address: &str,
    from_address: Option<&str>,
    private_key: Option<&str>,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    println!("\n=== Whitelisting member addresses ===\n");

    let member_signers = collect_member_rsk_addresses(member_funding_info);
    let unique = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure the coordinator and user-api services are running.",
            expected,
            unique.len()
        );
    }

    println!("Member addresses to whitelist:");
    for addr in &unique {
        println!("  {}", addr);
    }
    println!();

    let addr_array = format!("[{}]", unique.join(","));
    let rpc_url = env.rpc_url()?;

    match env {
        Environment::Local | Environment::Docker => {
            let from_address = resolve_local_whitelist_sender(from_address, private_key)?;
            println!(
                "Running: cast send --rpc-url {} --from {} {} \"whitelistAddresses(address[])\" \"{}\" --unlocked",
                rpc_url, from_address, contract_address, addr_array
            );

            let output = Command::new("cast")
                .arg("send")
                .arg("--rpc-url")
                .arg(&rpc_url)
                .arg("--from")
                .arg(from_address)
                .arg(contract_address)
                .arg("whitelistAddresses(address[])")
                .arg(&addr_array)
                .arg("--unlocked")
                .output()
                .context("failed to execute cast send for whitelistAddresses")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("whitelistAddresses transaction failed: {}", stderr.trim());
            }

            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
        Environment::Remote(_) => {
            let key = match resolve_remote_whitelist_private_key(from_address, private_key)? {
                Some(key) => key,
                None => {
                    let prompted = prompt_password("Enter Whitelister Private Key: ")
                        .context("failed to read private key")?
                        .to_string();
                    normalize_private_key(&prompted)
                        .context("private key is required for remote environments")?
                }
            };

            println!(
                "Running: cast send {} \"whitelistAddresses(address[])\" \"{}\" --private-key <REDACTED> --rpc-url {}",
                contract_address, addr_array, rpc_url
            );

            let output = Command::new("cast")
                .arg("send")
                .arg(contract_address)
                .arg("whitelistAddresses(address[])")
                .arg(&addr_array)
                .arg("--private-key")
                .arg(&key)
                .arg("--rpc-url")
                .arg(&rpc_url)
                .output()
                .context("failed to execute cast send for whitelistAddresses")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("whitelistAddresses transaction failed: {}", stderr.trim());
            }

            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }

    println!("Done. Whitelisted {} member addresses on CommitteeRegistry.", unique.len());

    Ok(())
}

fn resolve_local_whitelist_sender(
    from_address: Option<&str>,
    private_key: Option<&str>,
) -> Result<String> {
    if private_key.is_some() {
        bail!(
            "`--private-key` is not supported for `operator whitelist` in local/docker. Use `--from <address>` or rely on the default unlocked anvil account."
        );
    }

    let sender = from_address.unwrap_or(LOCAL_ANVIL_ADDRESS).trim();
    validate_address(sender).context("invalid `--from` address")?;
    Ok(sender.to_string())
}

fn resolve_remote_whitelist_private_key(
    from_address: Option<&str>,
    private_key: Option<&str>,
) -> Result<Option<String>> {
    if from_address.is_some() {
        bail!(
            "`--from` is only supported for `operator whitelist` in local/docker. Use `--private-key <hex-key>` in remote mode."
        );
    }

    private_key.map(normalize_private_key).transpose()
}

fn validate_address(address: &str) -> Result<()> {
    if !has_prefixed_hex_len(address, 40) {
        bail!("expected a 20-byte hex address with 0x prefix");
    }
    Ok(())
}

fn normalize_private_key(private_key: &str) -> Result<String> {
    let trimmed = private_key.trim();
    if trimmed.is_empty() {
        bail!("private key cannot be empty");
    }

    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("expected a 32-byte hex private key");
    }

    Ok(trimmed.to_string())
}

fn has_prefixed_hex_len(value: &str, hex_len: usize) -> bool {
    let Some(hex) = value.strip_prefix("0x") else {
        return false;
    };

    hex.len() == hex_len && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// handles funding rootstock wallets for operator stacks
pub(crate) async fn handle_operator_funding(
    env: Environment,
    stream_id: u64,
    stream_manager_address: Option<&str>,
    roles: Option<&str>,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    match env {
        Environment::Local => {
            fund_local(stream_id, member_funding_info).await?;
        }
        Environment::Docker => {
            fund_local_docker(stream_id, member_funding_info).await?;
        }
        Environment::Remote(_) => {
            print_instructions(&env, stream_id, stream_manager_address, roles, member_funding_info)
                .await?
        }
    }
    Ok(())
}

/// displays user addresses and funding instructions
pub(crate) async fn handle_user_funding(env: Environment) -> Result<()> {
    println!("\n=== User Funding Information ===\n");

    let user_addresses = collect_user_rsk_addresses(&env, false).await?;

    // print RSK funding instructions
    println!("--- Rootstock (RSK) ---");
    if user_addresses.is_empty() {
        println!("No user RSK addresses found in staged keystores.");
        println!("Ensure cli-setup-operators.sh has prepared ~/.union_bridge artifacts.\n");
    } else {
        println!("User RSK addresses to fund:");
        for (source, address) in &user_addresses {
            println!("  {} -> {}", source, address);
        }
        println!();

        let rpc_url = env.rpc_url()?;
        match env {
            Environment::Local | Environment::Docker => {
                println!("Fund using (local anvil):");
                println!(
                    "  value should be: pegout amount in wei + {} wei gas buffer",
                    LOCAL_USER_RSK_GAS_BUFFER_WEI
                );
                for (_, address) in &user_addresses {
                    println!(
                        "  cast send --rpc-url {} --from {} {} --value <AMOUNT_IN_WEI_PLUS_BUFFER> --unlocked",
                        rpc_url, LOCAL_ANVIL_ADDRESS, address
                    );
                }
            }
            Environment::Remote(_) => {
                println!("Fund with `cast` interactively using a key you control:");
                for (_, address) in &user_addresses {
                    println!(
                        "  cast send {} --value <VARIABLE_AMOUNT_PER_STREAM> --interactive --rpc-url {}",
                        address, rpc_url
                    );
                }
            }
        }
    }

    // print Bitcoin funding instructions
    println!("\n--- Bitcoin ---");
    match derive_user_bitcoin_address_from_env(&env) {
        Ok(address) => {
            println!("User Bitcoin address to fund: {}", address);
            println!();
            println!("Use your bitcoin-wallet CLI:");
            println!("  send_to_address <user_btc_address> [amount]");
            println!();
            println!("Note: Use the address of a Bitcoin private key you control");
        }
        Err(e) => {
            println!("Could not derive user Bitcoin address: {}", e);
            println!("Ensure USER_BITCOIN_WIF is exported in your shell.");
        }
    }

    Ok(())
}

/// returns the first user RSK address exposed by user-api for the current environment.
/// when `first_only` is true, only resolves the first configured endpoint (used for pegout)
pub(crate) async fn get_user_rsk_address(
    env: &Environment,
    first_only: bool,
) -> Result<Option<String>> {
    let addresses = collect_user_rsk_addresses(env, first_only).await?;
    Ok(addresses.into_iter().next().map(|(_, addr)| addr))
}

fn collect_member_rsk_addresses(
    member_funding_info: &CollectedMemberFundingInfo,
) -> Vec<(String, String)> {
    member_funding_info
        .iter()
        .map(|(endpoint, info)| (endpoint.clone(), info.rsk_address.clone()))
        .collect()
}

async fn collect_user_rsk_addresses(
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    collect_rsk_addresses_from_user_api(env, "/user/rsk-address", first_only).await
}

async fn collect_rsk_addresses_from_user_api(
    env: &Environment,
    path: &str,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let mut endpoints = env.user_api_endpoints()?;
    if first_only {
        endpoints.truncate(1);
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build http client")?;

    let mut addresses = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let url = format!("http://{}{}", endpoint, path);
        let response =
            client.get(&url).send().await.with_context(|| format!("failed to fetch {}", url))?;

        if !response.status().is_success() {
            bail!("request to {} failed with status {}", url, response.status());
        }

        let body: AddressResponse = response
            .json()
            .await
            .with_context(|| format!("failed to decode response body from {}", url))?;
        addresses.push((endpoint, body.address));
    }

    Ok(addresses)
}

async fn fund_local(
    stream_id: u64,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    println!("[cargo-fund] funding operator wallets via local anvil");
    let member_signers = collect_member_rsk_addresses(member_funding_info);
    let unique_members = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique_members.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure coordinator and user-api services are running.",
            expected,
            unique_members.len()
        );
    }

    let rpc_url = Environment::Local.rpc_url()?;

    for (index, (operator_id, address)) in member_signers.iter().enumerate() {
        println!("Processing coordinator-{}", operator_id);
        println!("  Funding member RSK address: {}", address);
        let required_balance = required_operator_rsk_balance(
            &rpc_url,
            LOCAL_STREAM_MANAGER_ADDRESS,
            stream_id,
            role_for_operator_index(index),
        )?;
        println!(
            "  Required RSK balance: {} RBTC ({} wei)",
            format_wei_as_rbtc(required_balance),
            required_balance
        );
        run_cast_send_local(address, required_balance)?;
    }

    println!("\n[cargo-fund] funding user wallets via local anvil");
    let user_signers = collect_user_rsk_addresses(&Environment::Local, false).await?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure user-api services are running.",
            expected,
            unique_users.len()
        );
    }

    let required_user_balance = required_user_rsk_balance(stream_id)?;

    for (operator_id, address) in &user_signers {
        println!("Processing user-api-{}", operator_id);
        println!("  Funding user RSK address: {}", address);
        println!(
            "  Required user RSK balance: {} RBTC ({} wei)",
            format_wei_as_rbtc(required_user_balance),
            required_user_balance
        );
        run_cast_send_local(address, required_user_balance)?;
    }

    println!("\nDone. Funded operator and user RSK addresses on local Anvil.");
    Ok(())
}

async fn fund_local_docker(
    stream_id: u64,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    println!("[docker-fund] funding operator wallets via local anvil");
    let member_signers = collect_member_rsk_addresses(member_funding_info);
    let unique_members = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique_members.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure coordinator and user-api services are running.",
            expected,
            unique_members.len()
        );
    }

    let rpc_url = Environment::Docker.rpc_url()?;

    for (index, (project, address)) in member_signers.iter().enumerate() {
        println!("Processing {}", project);
        println!("  Funding member RSK address: {}", address);
        let required_balance = required_operator_rsk_balance(
            &rpc_url,
            LOCAL_STREAM_MANAGER_ADDRESS,
            stream_id,
            role_for_operator_index(index),
        )?;
        println!(
            "  Required RSK balance: {} RBTC ({} wei)",
            format_wei_as_rbtc(required_balance),
            required_balance
        );
        run_cast_send_local(address, required_balance)?;
    }

    println!("\n[docker-fund] funding user wallets via local anvil");
    let user_signers = collect_user_rsk_addresses(&Environment::Docker, false).await?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure user-api services are running.",
            expected,
            unique_users.len()
        );
    }

    let required_user_balance = required_user_rsk_balance(stream_id)?;

    for (project, address) in &user_signers {
        println!("Processing {} (user)", project);
        println!("  Funding user RSK address: {}", address);
        println!(
            "  Required user RSK balance: {} RBTC ({} wei)",
            format_wei_as_rbtc(required_user_balance),
            required_user_balance
        );
        run_cast_send_local(address, required_user_balance)?;
    }

    println!("\nDone. Funded operator and user RSK addresses on local Anvil.");
    Ok(())
}

async fn print_instructions(
    env: &Environment,
    stream_id: u64,
    stream_manager_address: Option<&str>,
    roles: Option<&str>,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    let env_name = env.get_name();

    let hosts = env.hosts()?;
    let expected = hosts.len();
    let rpc_url = env.rpc_url()?;
    let stream_manager_address = stream_manager_address
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("--stream-manager-address is required for remote environments"))?;
    let roles = parse_remote_operator_roles(roles, expected)?;

    println!("[docker-fund] gathering operator wallets from coordinator APIs on {}", env_name);
    let signers = collect_member_rsk_addresses(member_funding_info);
    let unique = unique_addresses(&signers);
    let expected = hosts.len();
    if unique.len() < expected {
        bail!(
            "expected {} RSK address(es) but found {}. ensure each remote host exposes the member user-api endpoint.",
            expected,
            unique.len()
        );
    }

    println!("Operator RSK addresses to fund on {}:", env_name);
    for (index, address) in unique.iter().enumerate() {
        let required_balance = required_operator_rsk_balance(
            &rpc_url,
            stream_manager_address,
            stream_id,
            roles[index],
        )?;
        println!(
            "  operator -> {} [{}] (required: {} RBTC / {} wei)",
            address,
            roles[index].as_str(),
            format_wei_as_rbtc(required_balance),
            required_balance
        );
    }
    println!();

    println!("Fund with `cast` interactively using a key you control:");
    for (index, address) in unique.into_iter().enumerate() {
        let required_balance = required_operator_rsk_balance(
            &rpc_url,
            stream_manager_address,
            stream_id,
            roles[index],
        )?;
        println!(
            "  cast send {} --value {} --interactive --rpc-url {}",
            address, required_balance, rpc_url
        );
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum CommitteeFundingRole {
    Prover,
    Verifier,
}

impl CommitteeFundingRole {
    fn role_id(self) -> u8 {
        match self {
            CommitteeFundingRole::Prover => 1,
            CommitteeFundingRole::Verifier => 2,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            CommitteeFundingRole::Prover => "prover",
            CommitteeFundingRole::Verifier => "verifier",
        }
    }
}

fn role_for_operator_index(index: usize) -> CommitteeFundingRole {
    if index.is_multiple_of(2) {
        CommitteeFundingRole::Prover
    } else {
        CommitteeFundingRole::Verifier
    }
}

fn parse_remote_operator_roles(
    roles: Option<&str>,
    expected_count: usize,
) -> Result<Vec<CommitteeFundingRole>> {
    let roles = roles
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "--roles is required for remote environments; expected {} comma-separated role(s) like prover,prover,verifier,verifier",
                expected_count
            )
        })?;

    let parsed = roles
        .split(',')
        .map(str::trim)
        .enumerate()
        .map(|(index, role)| match role {
            "prover" => Ok(CommitteeFundingRole::Prover),
            "verifier" => Ok(CommitteeFundingRole::Verifier),
            _ => Err(anyhow!(
                "invalid remote role '{}' at position {}; expected 'prover' or 'verifier'",
                role,
                index + 1
            )),
        })
        .collect::<Result<Vec<_>>>()?;

    if parsed.len() != expected_count {
        bail!(
            "--roles count mismatch: expected {} role(s) but got {}",
            expected_count,
            parsed.len()
        );
    }

    Ok(parsed)
}

fn required_user_rsk_balance(stream_id: u64) -> Result<U256> {
    let amount_in_wei = derive_stream_funding_profile(
        stream_id,
        true,
        slots_per_package()?,
        committee_member_count()?,
        prover_count()?,
    )
    .map(|profile| U256::from(profile.denomination) * U256::from(WEI_PER_SAT))?;

    Ok(amount_in_wei + U256::from(LOCAL_USER_RSK_GAS_BUFFER_WEI))
}

fn unique_addresses(records: &[(String, String)]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();
    for (_, address) in records {
        if seen.insert(address.clone()) {
            unique.push(address.clone());
        }
    }
    unique
}

fn required_operator_rsk_balance(
    rpc_url: &str,
    stream_manager_address: &str,
    stream_id: u64,
    role: CommitteeFundingRole,
) -> Result<U256> {
    let min_deposit = fetch_stream_min_deposit(rpc_url, stream_manager_address, stream_id, role)?;
    Ok(required_member_rsk_balance(min_deposit, slots_per_package()?, committee_member_count()?))
}

fn fetch_stream_min_deposit(
    rpc_url: &str,
    stream_manager_address: &str,
    stream_id: u64,
    role: CommitteeFundingRole,
) -> Result<U256> {
    let output = Command::new("cast")
        .arg("call")
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg(stream_manager_address)
        .arg("getMinimumDeposit(uint8,uint8)(uint256)")
        .arg(stream_id.to_string())
        .arg(role.role_id().to_string())
        .output()
        .context("failed to execute cast call for getMinimumDeposit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("getMinimumDeposit call failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout).context("cast call output is not valid utf-8")?;
    parse_u256(stdout.trim()).context("failed to parse getMinimumDeposit response")
}

fn parse_u256(value: &str) -> Result<U256> {
    let normalized = value.split_whitespace().next().unwrap_or(value).trim();

    if let Some(hex) = normalized.strip_prefix("0x") {
        U256::from_str_radix(hex, 16)
            .map_err(|err| anyhow!("invalid hex uint256 '{}': {}", normalized, err))
    } else {
        U256::from_str(normalized)
            .map_err(|err| anyhow!("invalid decimal uint256 '{}': {}", normalized, err))
    }
}

fn format_wei_as_rbtc(value: U256) -> String {
    let whole = value / U256::from(WEI_PER_RBTC);
    let fractional = value % U256::from(WEI_PER_RBTC);

    if fractional.is_zero() {
        return whole.to_string();
    }

    let mut fractional_str = fractional.to_string();
    if fractional_str.len() < 18 {
        fractional_str = format!("{fractional_str:0>18}");
    }
    let trimmed = fractional_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
}

fn run_cast_send_local(address: &str, value: U256) -> Result<()> {
    let rpc_url = Environment::Local.rpc_url()?;
    eprintln!(
        "  Running: cast send --rpc-url {} --from {} {} --value {} --unlocked ({} RBTC)",
        rpc_url,
        LOCAL_ANVIL_ADDRESS,
        address,
        value,
        format_wei_as_rbtc(value)
    );
    let output = Command::new("cast")
        .arg("send")
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--from")
        .arg(LOCAL_ANVIL_ADDRESS)
        .arg(address)
        .arg("--value")
        .arg(value.to_string())
        .arg("--unlocked")
        .output()
        .context("failed to execute cast send")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cast send failed for {}: {}", address, stderr.trim());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_u256_decimal_and_hex() {
        assert_eq!(parse_u256("123").unwrap(), U256::from(123_u64));
        assert_eq!(parse_u256("0x7b").unwrap(), U256::from(123_u64));
        assert_eq!(
            parse_u256("25000000000000000 [2.5e16]").unwrap(),
            U256::from(25_000_000_000_000_000_u64)
        );
    }

    #[test]
    fn formats_wei_as_rbtc() {
        assert_eq!(format_wei_as_rbtc(U256::from(WEI_PER_RBTC)), "1");
        assert_eq!(format_wei_as_rbtc(U256::from(26_000_000_000_500_000_u64)), "0.0260000000005");
    }

    #[test]
    fn local_whitelist_uses_default_unlocked_sender() {
        let sender = resolve_local_whitelist_sender(None, None).unwrap();
        assert_eq!(sender, LOCAL_ANVIL_ADDRESS);
    }

    #[test]
    fn local_whitelist_rejects_private_key_flag() {
        let err = resolve_local_whitelist_sender(
            None,
            Some("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("`--private-key` is not supported"));
    }

    #[test]
    fn local_whitelist_validates_from_address() {
        let err = resolve_local_whitelist_sender(
            Some("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            None,
        )
        .unwrap_err();

        assert!(err.to_string().contains("invalid `--from` address"));
    }

    #[test]
    fn remote_whitelist_rejects_from_flag() {
        let err = resolve_remote_whitelist_private_key(
            Some("0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"),
            None,
        )
        .unwrap_err();

        assert!(err.to_string().contains("`--from` is only supported"));
    }

    #[test]
    fn remote_whitelist_accepts_private_key() {
        let key = resolve_remote_whitelist_private_key(
            None,
            Some("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .unwrap();

        assert_eq!(
            key.as_deref(),
            Some("0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn remote_whitelist_validates_private_key_format() {
        let err = resolve_remote_whitelist_private_key(
            None,
            Some("0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("expected a 32-byte hex private key"));
    }

    #[test]
    fn parses_remote_operator_roles() {
        let roles = parse_remote_operator_roles(Some("prover, prover, verifier"), 3).unwrap();
        assert!(matches!(roles[0], CommitteeFundingRole::Prover));
        assert!(matches!(roles[1], CommitteeFundingRole::Prover));
        assert!(matches!(roles[2], CommitteeFundingRole::Verifier));
    }

    #[test]
    fn remote_operator_roles_are_required() {
        let err = parse_remote_operator_roles(None, 2).unwrap_err();
        assert!(err.to_string().contains("--roles is required for remote environments"));
    }

    #[test]
    fn remote_operator_roles_validate_count() {
        let err = parse_remote_operator_roles(Some("prover,verifier"), 3).unwrap_err();
        assert!(err.to_string().contains("--roles count mismatch"));
    }

    #[test]
    fn remote_operator_roles_validate_values() {
        let err = parse_remote_operator_roles(Some("prover,operator"), 2).unwrap_err();
        assert!(err.to_string().contains("invalid remote role"));
    }
}
