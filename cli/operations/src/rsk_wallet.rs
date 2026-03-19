use anyhow::{anyhow, bail, Context, Result};
use key_manager::key_manager::KeyManager;
use rpassword::prompt_password;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::constants::{
    operator_ids, LOCAL_ANVIL_ADDRESS, ONE_OPERATOR_COMPOSE_PROJECT, REMOTE_SSH_USER,
};
use crate::environments::*;
use crate::utils::command_to_string;
use crate::validate_1_10;

const MEMBER_LOG_MARKER: &str = "Got member signer with address";
const USER_LOG_MARKER: &str = "Got user signer with address";
const USER_RSK_LOG_MARKER: &str = "Connected to Rootstock at";
const USER_RSK_ADDRESS_MARKER: &str = "as User with address";

/// whitelists member RSK addresses on the CommitteeRegistry contract.
/// collects member signer addresses from coordinator logs, then calls
/// `whitelistAddresses(address[])` via `cast send`.
pub fn handle_whitelist(
    env: Environment,
    contract_address: &str,
    from_address: Option<&str>,
    private_key: Option<&str>,
) -> Result<()> {
    println!("\n=== Whitelisting member addresses ===\n");

    let member_signers = match env {
        Environment::Local => collect_local_signers_from_logs(MEMBER_LOG_MARKER)?,
        Environment::LocalDocker => collect_local_signers(MEMBER_LOG_MARKER)?,
        Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
            collect_remote_member_addresses(&env.hosts())?
        }
    };

    let unique = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure all operator services are running and have emitted signer addresses.",
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
    let rpc_url = env.rpc_url();

    match env {
        Environment::Local | Environment::LocalDocker => {
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
        Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
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
            "`--private-key` is not supported for `operator whitelist` in local/local-docker. Use `--from <address>` or rely on the default unlocked anvil account."
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
            "`--from` is only supported for `operator whitelist` in local/local-docker. Use `--private-key <hex-key>` in regtest/alphanet/testnet."
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

/// handles creating local rootstock wallets for multi-client deployments
pub fn handle_wallet_creation(num_wallets: u8, base_storage_path: Option<&str>) -> Result<()> {
    let base = require_base_storage_path(base_storage_path)?;
    validate_1_10(num_wallets, "num-wallets")?;

    setup_wallets_create(num_wallets, base)?;
    print_wallet_summary("create", num_wallets);

    Ok(())
}

/// handles funding rootstock wallets for operator stacks
pub async fn handle_operator_funding(env: Environment) -> Result<()> {
    match env {
        Environment::Local => {
            fund_local()?;
        }
        Environment::LocalDocker => {
            fund_local_docker()?;
        }
        Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
            print_instructions(env)?;
        }
    }
    Ok(())
}

/// displays user addresses and funding instructions
pub fn handle_user_funding(env: Environment) -> Result<()> {
    println!("\n=== User Funding Information ===\n");

    // collect user RSK addresses from logs (all operators for funding display)
    let user_addresses = match env {
        Environment::Local => collect_user_rsk_addresses_from_cargo_logs(false)?,
        Environment::LocalDocker => collect_user_rsk_addresses_from_local_docker(false)?,
        Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
            collect_user_rsk_addresses_from_remote(env, false)?
        }
    };

    // print RSK funding instructions
    println!("--- Rootstock (RSK) ---");
    if user_addresses.is_empty() {
        println!("No user RSK addresses found in logs.");
        println!("Ensure user-api services are running and have emitted the connection log.\n");
    } else {
        println!("User RSK addresses to fund:");
        for (source, address) in &user_addresses {
            println!("  {} -> {}", source, address);
        }
        println!();

        let rpc_url = env.rpc_url();
        match env {
            Environment::Local | Environment::LocalDocker => {
                println!("Fund using (local anvil):");
                for (_, address) in &user_addresses {
                    println!(
                        "  cast send --rpc-url {} --from {} {} --value 1ether --unlocked",
                        rpc_url, LOCAL_ANVIL_ADDRESS, address
                    );
                }
            }
            Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
                let private_key = prompt_password("Enter Cow Private Key: ")
                    .context("failed to read private key")?
                    .trim()
                    .to_string();
                println!();

                if private_key.is_empty() {
                    bail!("private key is required");
                }

                println!("Fund using:");
                for (_, address) in &user_addresses {
                    println!(
                        "  cast send {} --value 0.25ether --private-key {} --rpc-url {}",
                        address, private_key, rpc_url
                    );
                }
            }
        }
    }

    // print Bitcoin funding instructions
    println!("\n--- Bitcoin ---");
    println!("Fund the Bitcoin address derived from the WIF key provided when starting user-api.");
    println!("Use your bitcoin-wallet CLI:");
    println!("  send_to_address <user_btc_address> [amount]");
    println!();
    println!("Note: Use the address of a Bitcoin private key you control");

    Ok(())
}

/// returns the first user RSK address found in logs (for the current environment)
/// when `first_only` is true, only queries operator 1 (used for pegout)
pub fn get_user_rsk_address(env: Environment, first_only: bool) -> Result<Option<String>> {
    let addresses = match env {
        Environment::Local => collect_user_rsk_addresses_from_cargo_logs(first_only)?,
        Environment::LocalDocker => collect_user_rsk_addresses_from_local_docker(first_only)?,
        Environment::Alphanet | Environment::Testnet | Environment::Regtest => {
            collect_user_rsk_addresses_from_remote(env, first_only)?
        }
    };
    Ok(addresses.into_iter().next().map(|(_, addr)| addr))
}

fn collect_user_rsk_addresses_from_cargo_logs(first_only: bool) -> Result<Vec<(String, String)>> {
    let logs_dir = cargo_logs_dir()?;
    let mut addresses = Vec::new();

    let all_ids = operator_ids();
    let ids: &[u8] = if first_only { &[1] } else { &all_ids };
    for operator_id in ids {
        let log_path = logs_dir.join(format!("user-api-{}.log", operator_id));
        if !log_path.exists() {
            continue;
        }

        let contents = fs::read_to_string(&log_path)
            .with_context(|| format!("failed to read {}", log_path.display()))?;

        if let Some(address) = extract_user_rsk_address(&contents) {
            addresses.push((format!("user-api-{}", operator_id), address));
        }
    }

    Ok(addresses)
}

fn collect_user_rsk_addresses_from_local_docker(first_only: bool) -> Result<Vec<(String, String)>> {
    let mut addresses = Vec::new();

    let all_ids = operator_ids();
    let ids: &[u8] = if first_only { &[1] } else { &all_ids };
    for id in ids {
        let project = format!("op_{}", id);
        let output = Command::new("docker")
            .args(["compose", "-p", &project, "logs", "user-api"])
            .output()
            .with_context(|| {
                format!("failed to run `docker compose -p {} logs user-api`", &project)
            })?;

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8(output.stdout)
            .context("docker compose logs output is not valid utf-8")?;

        if let Some(address) = extract_user_rsk_address(&stdout) {
            addresses.push((project, address));
        }
    }

    Ok(addresses)
}

fn collect_user_rsk_addresses_from_remote(
    env: Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let all_hosts = env.hosts();
    let hosts: Vec<&String> = if first_only {
        all_hosts.first().into_iter().collect()
    } else {
        all_hosts.iter().collect()
    };
    let mut addresses = Vec::new();

    for host in hosts {
        let target = format!("{}@{}", REMOTE_SSH_USER, host);

        let mut cmd = Command::new("ssh");
        cmd.arg(&target).args([
            "docker",
            "compose",
            "-p",
            ONE_OPERATOR_COMPOSE_PROJECT,
            "logs",
            "user-api",
        ]);

        let cmd_str = command_to_string(&cmd);
        println!("{}", cmd_str);

        let output = cmd.output().with_context(|| format!("failed to run `{}`", cmd_str))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("[user-fund] ssh command failed for {}: {}", host, stderr.trim());
            continue;
        }

        let stdout = String::from_utf8(output.stdout).context("ssh output is not valid utf-8")?;

        if let Some(address) = extract_user_rsk_address(&stdout) {
            addresses.push((host.to_string(), address));
        } else {
            println!("[user-fund] no user RSK address found on host {}", host);
        }
    }

    Ok(addresses)
}

fn extract_user_rsk_address(log_content: &str) -> Option<String> {
    // pattern: "Connected to Rootstock at <url> as User with address <address>"
    for line in log_content.lines().rev() {
        if line.contains(USER_RSK_LOG_MARKER) && line.contains(USER_RSK_ADDRESS_MARKER) {
            if let Some(idx) = line.find(USER_RSK_ADDRESS_MARKER) {
                let after = &line[idx + USER_RSK_ADDRESS_MARKER.len()..];
                if let Some(addr) = after.split_whitespace().find(|s| s.starts_with("0x")) {
                    return Some(addr.trim().to_string());
                }
            }
        }
    }
    None
}

fn require_base_storage_path(base_storage_path: Option<&str>) -> Result<&str> {
    base_storage_path.ok_or_else(|| {
        anyhow!(
            "BASE_STORAGE_PATH environment variable is required (e.g., export BASE_STORAGE_PATH=/Users/username)"
        )
    })
}

fn print_wallet_summary(mode: &str, num_wallets: u8) {
    println!("\n=== wallet setup summary ===");
    println!("setup mode: {}", mode);
    println!("number of clients: {}", num_wallets);
    println!("total wallets: {} (member + user per client)", num_wallets * 2);
}

fn create_or_use_keystore(keystore_path: &Path, file_name: &str, password: &str) -> Result<()> {
    let full_keystore_path = keystore_path.join(file_name);

    if full_keystore_path.exists() {
        println!(
            "[wallet-setup] key already exists at {}, skipping generation",
            full_keystore_path.display()
        );
        return Ok(());
    }

    println!("[wallet-setup] creating new key at {}...", keystore_path.display());

    // create directory if it doesn't exist
    fs::create_dir_all(keystore_path)
        .with_context(|| format!("failed to create directory {}", keystore_path.display()))?;

    // generate key using KeyManager directly
    let (generated_path, _public_key, _address) = KeyManager::generate_key(keystore_path, password)
        .context("failed to generate key with KeyManager")?;

    // rename the generated key to the desired filename
    fs::rename(&generated_path, &full_keystore_path).with_context(|| {
        format!("failed to rename {} to {}", generated_path, full_keystore_path.display())
    })?;

    println!("[wallet-setup] key created successfully at {}", full_keystore_path.display());

    Ok(())
}

fn setup_wallets_create(num_wallets: u8, base_storage_path: &str) -> Result<()> {
    let password = std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable is required")?;

    let keystore_base_path =
        PathBuf::from(base_storage_path).join(".union_bridge").join("keystore");

    println!("[wallet-setup] starting wallet creation...");

    for i in 1..=num_wallets {
        // create member wallet
        let member_name = format!("multi-client-{}-member", i);
        create_or_use_keystore(&keystore_base_path, &member_name, &password)
            .with_context(|| format!("failed to create member wallet for client {}", i))?;

        // create user wallet
        let user_name = format!("multi-client-{}-user", i);
        create_or_use_keystore(&keystore_base_path, &user_name, &password)
            .with_context(|| format!("failed to create user wallet for client {}", i))?;
    }

    println!("[wallet-setup] wallet creation complete! all keystores have been created.");
    Ok(())
}

fn fund_local() -> Result<()> {
    println!("[cargo-fund] funding operator wallets via local anvil");
    let member_signers = collect_local_signers_from_logs(MEMBER_LOG_MARKER)?;
    let unique_members = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique_members.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure all required operator services are running and have emitted signer addresses.",
            expected,
            unique_members.len()
        );
    }

    for (operator_id, address) in &member_signers {
        println!("Processing coordinator-{}", operator_id);
        println!("  Funding member RSK address: {}", address);
        run_cast_send_local(&address)?;
    }

    println!("\n[cargo-fund] funding user wallets via local anvil");
    let user_signers = collect_local_signers_from_logs(USER_LOG_MARKER)?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure all required operator services are running and have emitted signer addresses.",
            expected,
            unique_users.len()
        );
    }

    for (operator_id, address) in &user_signers {
        println!("Processing user-api-{}", operator_id);
        println!("  Funding user RSK address: {}", address);
        run_cast_send_local(&address)?;
    }

    println!("\nDone. Funded operator and user RSK addresses on local Anvil.");
    Ok(())
}

fn fund_local_docker() -> Result<()> {
    println!("[docker-fund] funding operator wallets via local anvil");
    let member_signers = collect_local_signers(MEMBER_LOG_MARKER)?;
    let unique_members = unique_addresses(&member_signers);
    let expected = operator_ids().len();
    if unique_members.len() < expected {
        bail!(
            "expected {} member RSK address(es) but found {}. ensure all required operator stacks are running and have emitted signer addresses.",
            expected,
            unique_members.len()
        );
    }

    for (project, address) in &member_signers {
        println!("Processing {}", project);
        println!("  Funding member RSK address: {}", address);
        run_cast_send_local(&address)?;
    }

    println!("\n[docker-fund] funding user wallets via local anvil");
    let user_signers = collect_local_signers(USER_LOG_MARKER)?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure all required operator stacks are running and have emitted signer addresses.",
            expected,
            unique_users.len()
        );
    }

    for (project, address) in &user_signers {
        println!("Processing {} (user)", project);
        println!("  Funding user RSK address: {}", address);
        run_cast_send_local(&address)?;
    }

    println!("\nDone. Funded operator and user RSK addresses on local Anvil.");
    Ok(())
}

fn print_instructions(env: Environment) -> Result<()> {
    let env_name = env.get_name();

    let hosts = env.hosts();
    let rpc_url = env.rpc_url();

    println!("[docker-fund] gathering operator wallets from {} hosts", env_name);
    let signers = collect_remote_member_addresses(&hosts)?;
    let unique = unique_addresses(&signers);
    let expected = hosts.len();
    if unique.len() < expected {
        bail!(
            "expected {} RSK address(es) but found {}. ensure all remote operator stacks are running and have emitted signer addresses.",
            expected,
            unique.len()
        );
    }

    println!("Operator RSK addresses to fund on {}:", env_name);
    for address in &unique {
        println!("  operator -> {}", address);
    }
    println!();

    let private_key = prompt_password("Enter Cow Private Key: ")
        .context("failed to read private key")?
        .trim()
        .to_string();
    println!();

    if private_key.is_empty() {
        bail!("private key is required");
    }

    println!("Fund using:");
    for address in unique {
        println!(
            "  cast send {} --value 0.25ether --private-key {} --rpc-url {}",
            address, private_key, rpc_url
        );
    }

    Ok(())
}

fn collect_local_signers_from_logs(marker: &str) -> Result<Vec<(String, String)>> {
    let logs_dir = cargo_logs_dir()?;
    let mut signers = Vec::new();

    let log_type = if marker == MEMBER_LOG_MARKER { "coordinator" } else { "user-api" };

    for operator_id in operator_ids() {
        let log_path = logs_dir.join(format!("{}-{}.log", log_type, operator_id));
        if !log_path.exists() {
            bail!(
                "expected {} log at {} but it does not exist. ensure the services have been started via `cargo run -- run`.",
                log_type,
                log_path.display()
            );
        }

        let contents = fs::read_to_string(&log_path)
            .with_context(|| format!("failed to read {}", log_path.display()))?;
        let mut addresses = extract_signer_addresses(&contents, marker);
        if addresses.is_empty() {
            bail!(
                "no {} signer addresses found in {}. wait for the {} to emit the log line and try again.",
                log_type,
                log_path.display(),
                log_type
            );
        } else {
            for address in addresses.drain(..) {
                println!("cargo {}-{} -> {}", log_type, operator_id, address);
                signers.push((operator_id.to_string(), address));
            }
        }
    }

    Ok(signers)
}

fn collect_local_signers(marker: &str) -> Result<Vec<(String, String)>> {
    let mut signers = Vec::new();
    let address_type = if marker == MEMBER_LOG_MARKER { "member" } else { "user" };

    for id in operator_ids() {
        let project = format!("op_{}", id);
        eprintln!("[docker-fund] running: docker compose -p {} logs", &project);
        let output = Command::new("docker")
            .args(["compose", "-p", &project, "logs"])
            .output()
            .with_context(|| format!("failed to run `docker compose -p {} logs`", &project))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("`docker compose -p {} logs` failed with: {}", &project, stderr.trim());
        }
        let stdout = String::from_utf8(output.stdout)
            .context("docker compose logs output is not valid utf-8")?;
        let mut addresses = extract_signer_addresses(&stdout, marker);
        if addresses.is_empty() {
            println!(
                "[docker-fund] no {} signer addresses found for project {}",
                address_type, project
            );
        } else {
            for address in addresses.drain(..) {
                signers.push((project.to_string(), address));
            }
        }
    }

    Ok(signers)
}

fn collect_remote_member_addresses(hosts: &[String]) -> Result<Vec<(String, String)>> {
    let mut signers = Vec::new();
    for host in hosts {
        let target = format!("{}@{}", REMOTE_SSH_USER, host);

        let mut cmd = Command::new("ssh");
        cmd.arg(&target).args(["docker", "compose", "-p", ONE_OPERATOR_COMPOSE_PROJECT, "logs"]);

        let cmd_str = command_to_string(&cmd);
        println!("{}", cmd_str);

        let output = cmd.output().with_context(|| format!("failed to run `{}`", cmd_str))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("`{}` failed with: {}", cmd_str, stderr.trim());
        }
        let stdout = String::from_utf8(output.stdout).context("ssh output is not valid utf-8")?;
        let mut addresses = extract_signer_addresses(&stdout, MEMBER_LOG_MARKER);
        if addresses.is_empty() {
            println!("[docker-fund] no signer addresses found on host {}", host);
        } else {
            for address in addresses.drain(..) {
                signers.push((host.to_string(), address));
            }
        }
    }

    Ok(signers)
}

fn extract_signer_addresses(log_content: &str, marker: &str) -> Vec<String> {
    let mut unique = HashSet::new();
    for line in log_content.lines() {
        if let Some(idx) = line.find(marker) {
            let after_marker = &line[idx + marker.len()..];
            if let Some(candidate) =
                after_marker.split_whitespace().find(|token| token.starts_with("0x"))
            {
                let cleaned = candidate
                    .trim_end_matches(|c: char| c == ',' || c == ';' || c == '.')
                    .to_string();
                unique.insert(cleaned);
            }
        }
    }

    let mut addresses: Vec<String> = unique.into_iter().collect();
    addresses.sort();
    addresses
}

fn unique_addresses(records: &[(String, String)]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for (_, address) in records {
        if seen.insert(address.clone()) {
            unique.push(address.clone());
        }
    }
    unique
}

fn cargo_logs_dir() -> Result<PathBuf> {
    let operations_cli_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project_root = operations_cli_dir
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow!("failed to resolve project root"))?;
    Ok(project_root.join("logs"))
}

fn run_cast_send_local(address: &str) -> Result<()> {
    let rpc_url = Environment::Local.rpc_url();
    eprintln!(
        "  Running: cast send --rpc-url {} --from {} {} --value 1ether --unlocked",
        rpc_url, LOCAL_ANVIL_ADDRESS, address
    );
    let output = Command::new("cast")
        .arg("send")
        .arg("--rpc-url")
        .arg(rpc_url)
        .arg("--from")
        .arg(LOCAL_ANVIL_ADDRESS)
        .arg(address)
        .arg("--value")
        .arg("2ether")
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
}
