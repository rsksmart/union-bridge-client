use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use bitcoin::address::Address as BitcoinAddress;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{CompressedPublicKey, Network, NetworkKind, PrivateKey};
use protocol_params::{committee_member_count, prover_count, slots_per_package};

use crate::constants::{operator_ids, UNION_BRIDGE_DIR};
use crate::environments::*;
use crate::member_funding_info::CollectedMemberFundingInfo;
use crate::utils::command_to_string;
use op_funding::derive_stream_funding_profile;

pub async fn handle_bitcoin_funding(
    environment: Environment,
    stream_id: u64,
    execute: bool,
    amount_override: Option<u64>,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<()> {
    if execute && environment.is_remote() {
        bail!("--execute flag is only supported for local environments (`local`/`docker`). For remote environments, please run the wallet commands manually.");
    }

    let funding_profile = derive_stream_funding_profile(
        stream_id,
        matches!(environment, Environment::Local | Environment::Docker),
        slots_per_package()?,
        committee_member_count()?,
        prover_count()?,
    )?;
    let amount = amount_override.unwrap_or(funding_profile.operator_fund_amount);

    let addresses = collect_addresses(&environment, member_funding_info)?;

    if addresses.is_empty() {
        bail!("no BitVMX funding addresses were discovered");
    }

    // Add a small fixed buffer so the wallet's `mine_utxo` amount still covers
    // the subsequent `send_to_address` transaction fee on regtest.
    let funding_utxo = amount
        .checked_mul(addresses.len() as u64)
        .and_then(|value| value.checked_add(10_000))
        .context("failed to compute wallet funding UTXO amount")?;

    println!();
    println!(
        "Derived stream {} funding: denomination={} protocol_funding={} speed_up_utxo={} advance_funds={} operator_fund_amount={}",
        stream_id,
        funding_profile.denomination,
        funding_profile.protocol_funding,
        funding_profile.speed_up_utxo,
        funding_profile.advance_funds,
        amount
    );

    if execute {
        println!("Executing wallet commands programmatically...");
        println!();
        execute_wallet_command(&addresses, amount)?;
    } else {
        print_instructions(&environment, &addresses, amount, funding_utxo);
    }

    Ok(())
}

fn collect_addresses(
    environment: &Environment,
    member_funding_info: &CollectedMemberFundingInfo,
) -> Result<Vec<String>> {
    let endpoints = environment.user_api_endpoints()?;
    println!("Fetching member funding info from: {} ...", endpoints.join(", "));
    let mut addresses = Vec::new();
    for (endpoint, info) in member_funding_info {
        println!("{} -> BTC {} / RSK {}", endpoint, info.bitcoin_address, info.rsk_address);
        addresses.push(info.bitcoin_address.clone());
    }

    Ok(addresses)
}

fn print_instructions(env: &Environment, addresses: &[String], amount: u64, funding_utxo: u64) {
    let joined = addresses.join(",");
    println!("Note: See the bitcoin-wallet README for how to start and use the CLI: ../cli/bitcoin-wallet/README.md\n");

    match env {
        Environment::Remote(_) => {
            println!(
                "Run the following command in your bitcoin-wallet or wallet tooling for {}:",
                env.get_name()
            );
            println!("  send_to_address {} {}\n", joined, amount);
        }
        Environment::Docker | Environment::Local => {
            println!("Run the following commands in the bitcoin-wallet CLI (Regtest):");
            println!("1 =>    clear_db   (if you see a misaligned utxos error)");
            println!("2 =>    mine_utxo {}", funding_utxo);
            println!("3 =>    send_to_address {} {}", joined, amount);
            println!("4 =>    mine_block");
        }
    }
}

fn execute_wallet_command(addresses: &[String], amount: u64) -> Result<()> {
    let wallet_script = "./cli-bitcoin-wallet.sh";
    let joined = addresses.join(",");
    let amount_str = amount.to_string();

    // just send to addresses - utxo mining and block mining handled externally
    let mut cmd = Command::new(wallet_script);
    cmd.arg("member").arg("send_to_address").arg(&joined).arg(&amount_str);

    println!("Running: {} member send_to_address {} {}", wallet_script, joined, amount);

    let output = cmd.output().context("failed to execute cli-bitcoin-wallet.sh")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "wallet command failed with status {}:\nstdout: {}\nstderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);

    Ok(())
}

pub fn collect_user_bitcoin_addresses(
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    match env {
        Environment::Local | Environment::Docker => {
            collect_local_user_bitcoin_addresses(env, first_only)
        }
        Environment::Remote(_) => collect_remote_user_bitcoin_addresses(env, first_only),
    }
}

fn collect_local_user_bitcoin_addresses(
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let storage_root = local_storage_root()?;
    let mut addresses = Vec::new();

    for operator_id in selected_local_operator_ids(first_only) {
        let wif = resolve_local_env_value(&storage_root, operator_id, "USER_BITCOIN_WIF")?;
        let address = derive_user_bitcoin_address(env, &wif)?;
        addresses.push((format!("op_{operator_id}"), address));
    }

    Ok(addresses)
}

fn collect_remote_user_bitcoin_addresses(
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let ssh_user = env.remote_ssh_user()?;
    let hosts = selected_remote_hosts(env, &ssh_user, first_only)?;
    let mut addresses = Vec::new();

    for (operator_id, host) in hosts {
        let target = format!("{ssh_user}@{host}");
        let wif = resolve_remote_env_value(&target, operator_id, "USER_BITCOIN_WIF")?;
        let address = derive_user_bitcoin_address(env, &wif)?;
        addresses.push((host, address));
    }

    Ok(addresses)
}

fn selected_local_operator_ids(first_only: bool) -> Vec<u8> {
    if first_only {
        vec![1]
    } else {
        operator_ids()
    }
}

fn selected_remote_hosts(
    env: &Environment,
    ssh_user: &str,
    first_only: bool,
) -> Result<Vec<(u8, String)>> {
    let hosts = env.hosts()?;
    if hosts.is_empty() {
        bail!("remote profile must define at least one host");
    }

    let items = if first_only { hosts.into_iter().take(1).collect() } else { hosts };

    items
        .into_iter()
        .map(|host| {
            let target = format!("{ssh_user}@{host}");
            let operator_id = discover_remote_operator_id(&target)?;
            Ok((operator_id, host))
        })
        .collect()
}

fn local_storage_root() -> Result<PathBuf> {
    if let Ok(value) = env::var("BASE_STORAGE_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let home = env::var("HOME").context(format!(
        "BASE_STORAGE_PATH is not set and HOME is unavailable; cannot locate ~/{UNION_BRIDGE_DIR}"
    ))?;
    Ok(PathBuf::from(home))
}

fn operator_runtime_env_path(storage_root: &Path, operator_id: u8) -> PathBuf {
    storage_root.join(UNION_BRIDGE_DIR).join(format!("op_{operator_id}")).join("docker-service.env")
}

fn remote_runtime_env_path(operator_id: u8) -> String {
    format!("~/{UNION_BRIDGE_DIR}/op_{operator_id}/docker-service.env")
}

fn resolve_local_env_value(storage_root: &Path, operator_id: u8, key: &str) -> Result<String> {
    let env_path = operator_runtime_env_path(storage_root, operator_id);
    let contents = fs::read_to_string(&env_path)
        .with_context(|| format!("failed to read {}", env_path.display()))?;

    lookup_key_in_env_contents(&contents, key)
        .ok_or_else(|| anyhow!("{key} is missing in {}", env_path.display()))
}

fn resolve_remote_env_value(target: &str, operator_id: u8, key: &str) -> Result<String> {
    let env_path = remote_runtime_env_path(operator_id);
    let contents =
        run_ssh_capture(target, &format!("cat {env_path}"), &format!("read {env_path}"))?;

    lookup_key_in_env_contents(&contents, key)
        .ok_or_else(|| anyhow!("{key} is missing in {env_path} on {target}"))
}

fn derive_user_bitcoin_address(env: &Environment, wif: &str) -> Result<String> {
    let private_key = PrivateKey::from_wif(wif).context("failed to parse USER_BITCOIN_WIF")?;
    let network = bitcoin_network_for_environment(env, private_key.network);
    let public_key = private_key.public_key(&Secp256k1::new());
    let compressed = CompressedPublicKey::try_from(public_key)
        .map_err(|_| anyhow!("USER_BITCOIN_WIF must correspond to a compressed public key"))?;
    let address = BitcoinAddress::p2wpkh(&compressed, network);
    Ok(address.to_string())
}

fn bitcoin_network_for_environment(env: &Environment, network_kind: NetworkKind) -> Network {
    match env {
        Environment::Local | Environment::Docker => Network::Regtest,
        Environment::Remote(profile) => match network_kind {
            NetworkKind::Main => Network::Bitcoin,
            NetworkKind::Test => {
                let lower = profile.to_ascii_lowercase();
                if lower.contains("signet") {
                    Network::Signet
                } else if lower.contains("regtest") {
                    Network::Regtest
                } else {
                    Network::Testnet
                }
            }
        },
    }
}

fn run_ssh_capture(target: &str, remote_command: &str, action: &str) -> Result<String> {
    let mut cmd = Command::new("ssh");
    cmd.arg(target).args(["sh", "-lc", remote_command]);

    let cmd_str = command_to_string(&cmd);
    println!("{}", cmd_str);

    let output = cmd.output().with_context(|| format!("failed to run `{cmd_str}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{action} failed via `{cmd_str}`: {}", stderr.trim());
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("`{cmd_str}` output is not valid utf-8"))
}

fn discover_remote_operator_id(target: &str) -> Result<u8> {
    let listing = run_ssh_capture(
        target,
        &format!(
            "for dir in ~/{UNION_BRIDGE_DIR}/op_*; do [ -d \"$dir\" ] && basename \"$dir\"; done"
        ),
        "list staged operator directories",
    )?;

    let operator_ids = parse_remote_operator_ids(&listing);
    match operator_ids.as_slice() {
        [operator_id] => Ok(*operator_id),
        [] => bail!("no staged operator directories found under ~/{UNION_BRIDGE_DIR} on {target}"),
        _ => bail!(
            "expected exactly one staged operator directory under ~/{UNION_BRIDGE_DIR} on {target}, found {:?}",
            operator_ids
        ),
    }
}

fn parse_remote_operator_ids(listing: &str) -> Vec<u8> {
    let mut operator_ids: Vec<u8> = listing
        .lines()
        .filter_map(|line| line.trim().strip_prefix("op_"))
        .filter_map(|value| value.parse::<u8>().ok())
        .collect();
    operator_ids.sort_unstable();
    operator_ids.dedup();
    operator_ids
}

fn lookup_key_in_env_contents(contents: &str, key: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((raw_key, raw_value)) = stripped.split_once('=') else {
            continue;
        };
        if raw_key.trim() != key {
            continue;
        }

        let value = raw_value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')))
            .unwrap_or(value)
            .trim();

        if !unquoted.is_empty() {
            return Some(unquoted.to_string());
        }
    }

    None
}
