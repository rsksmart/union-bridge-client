use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use bitcoin::address::Address as BitcoinAddress;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{CompressedPublicKey, Network, NetworkKind, PrivateKey};
use key_manager::key_manager::KeyManager;
use tempfile::NamedTempFile;

use crate::constants::operator_ids;
use crate::environments::Environment;
use crate::utils::command_to_string;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RskRole {
    User,
    Member,
}

impl RskRole {
    fn keystore_name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Member => "member",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Member => "member",
        }
    }
}

pub fn collect_rsk_addresses(
    env: &Environment,
    role: RskRole,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    match env {
        Environment::Local | Environment::Docker => collect_local_rsk_addresses(role, first_only),
        Environment::Remote(_) => collect_remote_rsk_addresses(env, role, first_only),
    }
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

fn collect_local_rsk_addresses(role: RskRole, first_only: bool) -> Result<Vec<(String, String)>> {
    let storage_root = local_storage_root()?;
    let mut addresses = Vec::new();

    for operator_id in selected_local_operator_ids(first_only) {
        let password = resolve_local_key_store_password(&storage_root, operator_id)?;
        let key_path = local_keystore_path(&storage_root, operator_id, role);
        let (_, address) = KeyManager::derive_public_key_and_address(&key_path, &password)
            .with_context(|| {
                format!(
                    "failed to derive {} RSK address from {}",
                    role.display_name(),
                    key_path.display()
                )
            })?;

        addresses.push((format!("op_{operator_id}"), prefixed_hex_address(&address)));
    }

    Ok(addresses)
}

fn collect_remote_rsk_addresses(
    env: &Environment,
    role: RskRole,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let ssh_user = env.remote_ssh_user()?;
    let hosts = selected_remote_hosts(env, first_only)?;
    let mut addresses = Vec::new();

    for (operator_id, host) in hosts {
        let target = format!("{ssh_user}@{host}");
        let password = resolve_remote_key_store_password(&target, operator_id)?;
        let key_path = remote_keystore_path(operator_id, role);
        let key_contents =
            run_ssh_capture(&target, &format!("cat {key_path}"), &format!("read {key_path}"))?;
        let temp_file = write_temp_keystore(&key_contents, &target, &key_path)?;

        let (_, address) = KeyManager::derive_public_key_and_address(temp_file.path(), &password)
            .with_context(|| {
            format!(
                "failed to derive {} RSK address from {} on {}",
                role.display_name(),
                key_path,
                host
            )
        })?;

        addresses.push((host, prefixed_hex_address(&address)));
    }

    Ok(addresses)
}

fn collect_local_user_bitcoin_addresses(
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let storage_root = local_storage_root()?;
    let mut addresses = Vec::new();

    for operator_id in selected_local_operator_ids(first_only) {
        let wif = resolve_local_user_bitcoin_wif(&storage_root, operator_id)?;
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
    let hosts = selected_remote_hosts(env, first_only)?;
    let mut addresses = Vec::new();

    for (operator_id, host) in hosts {
        let target = format!("{ssh_user}@{host}");
        let wif = resolve_remote_user_bitcoin_wif(&target, operator_id)?;
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

fn selected_remote_hosts(env: &Environment, first_only: bool) -> Result<Vec<(u8, String)>> {
    let hosts = env.hosts()?;
    if hosts.is_empty() {
        bail!("remote profile must define at least one host");
    }

    let items = if first_only { hosts.into_iter().take(1).collect() } else { hosts };

    items
        .into_iter()
        .enumerate()
        .map(|(idx, host)| {
            let operator_id = u8::try_from(idx + 1).context("remote operator index overflowed")?;
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

    let home = env::var("HOME").context(
        "BASE_STORAGE_PATH is not set and HOME is unavailable; cannot locate ~/.union_bridge",
    )?;
    Ok(PathBuf::from(home))
}

fn local_keystore_path(storage_root: &Path, operator_id: u8, role: RskRole) -> PathBuf {
    storage_root
        .join(".union_bridge")
        .join(format!("op_{operator_id}"))
        .join("union-client/keystore")
        .join(role.keystore_name())
}

fn remote_keystore_path(operator_id: u8, role: RskRole) -> String {
    format!("~/.union_bridge/op_{operator_id}/union-client/keystore/{}", role.keystore_name())
}

fn operator_runtime_env_path(storage_root: &Path, operator_id: u8) -> PathBuf {
    storage_root.join(".union_bridge").join(format!("op_{operator_id}")).join("docker-service.env")
}

fn remote_runtime_env_path(operator_id: u8) -> String {
    format!("~/.union_bridge/op_{operator_id}/docker-service.env")
}

fn resolve_local_key_store_password(storage_root: &Path, operator_id: u8) -> Result<String> {
    if let Some(value) = env_var_if_non_empty("KEY_STORE_PASSWORD") {
        return Ok(value);
    }

    let env_path = operator_runtime_env_path(storage_root, operator_id);
    let contents = fs::read_to_string(&env_path)
        .with_context(|| format!("failed to read {}", env_path.display()))?;

    lookup_key_in_env_contents(&contents, "KEY_STORE_PASSWORD")
        .ok_or_else(|| anyhow!("KEY_STORE_PASSWORD is missing in {}", env_path.display()))
}

fn resolve_remote_key_store_password(target: &str, operator_id: u8) -> Result<String> {
    if let Some(value) = env_var_if_non_empty("KEY_STORE_PASSWORD") {
        return Ok(value);
    }

    let env_path = remote_runtime_env_path(operator_id);
    let contents =
        run_ssh_capture(target, &format!("cat {env_path}"), &format!("read {env_path}"))?;

    lookup_key_in_env_contents(&contents, "KEY_STORE_PASSWORD")
        .ok_or_else(|| anyhow!("KEY_STORE_PASSWORD is missing in {env_path} on {target}"))
}

fn resolve_local_user_bitcoin_wif(storage_root: &Path, operator_id: u8) -> Result<String> {
    if let Some(value) = env_var_if_non_empty("USER_BITCOIN_WIF") {
        return Ok(value);
    }

    let env_path = operator_runtime_env_path(storage_root, operator_id);
    let contents = fs::read_to_string(&env_path)
        .with_context(|| format!("failed to read {}", env_path.display()))?;

    lookup_key_in_env_contents(&contents, "USER_BITCOIN_WIF")
        .ok_or_else(|| anyhow!("USER_BITCOIN_WIF is missing in {}", env_path.display()))
}

fn resolve_remote_user_bitcoin_wif(target: &str, operator_id: u8) -> Result<String> {
    if let Some(value) = env_var_if_non_empty("USER_BITCOIN_WIF") {
        return Ok(value);
    }

    let env_path = remote_runtime_env_path(operator_id);
    let contents =
        run_ssh_capture(target, &format!("cat {env_path}"), &format!("read {env_path}"))?;

    lookup_key_in_env_contents(&contents, "USER_BITCOIN_WIF")
        .ok_or_else(|| anyhow!("USER_BITCOIN_WIF is missing in {env_path} on {target}"))
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

fn write_temp_keystore(contents: &str, target: &str, source_path: &str) -> Result<NamedTempFile> {
    let mut temp_file = NamedTempFile::new().context("failed to create temporary keystore file")?;
    use std::io::Write;
    temp_file
        .write_all(contents.as_bytes())
        .with_context(|| format!("failed to stage remote keystore {source_path} from {target}"))?;
    Ok(temp_file)
}

fn lookup_key_in_env_contents(contents: &str, key: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let (raw_key, raw_value) = stripped.split_once('=')?;
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

fn env_var_if_non_empty(key: &str) -> Option<String> {
    env::var(key).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn prefixed_hex_address(address: &str) -> String {
    if address.starts_with("0x") {
        address.to_string()
    } else {
        format!("0x{address}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exported_env_values() {
        let value = lookup_key_in_env_contents(
            "export KEY_STORE_PASSWORD=\"secret\"\n",
            "KEY_STORE_PASSWORD",
        );
        assert_eq!(value.as_deref(), Some("secret"));
    }

    #[test]
    fn prefers_testnet_for_remote_test_profiles() {
        let network = bitcoin_network_for_environment(
            &Environment::Remote("alphanet".to_string()),
            NetworkKind::Test,
        );
        assert_eq!(network, Network::Testnet);
    }

    #[test]
    fn local_and_docker_user_bitcoin_use_regtest_addresses() {
        let network = bitcoin_network_for_environment(&Environment::Local, NetworkKind::Test);
        assert_eq!(network, Network::Regtest);
    }
}
