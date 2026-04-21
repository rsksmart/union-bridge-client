use alloy_primitives::U256;
use anyhow::{anyhow, bail, Context, Result};
use rpassword::prompt_password;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use op_funding::{derive_stream_funding_profile, required_member_rsk_balance};

use crate::constants::{
    operator_and_prover_counts, operator_ids, COMMITTEE_PACKET_SIZE, LOCAL_ANVIL_ADDRESS,
    ONE_OPERATOR_COMPOSE_PROJECT,
};
use crate::environments::*;
use crate::utils::command_to_string;

const MEMBER_LOG_MARKER: &str = "Got member signer with address";
const USER_LOG_MARKER: &str = "Got user signer with address";
const USER_RSK_LOG_MARKER: &str = "Connected to Rootstock at";
const USER_RSK_ADDRESS_MARKER: &str = "as User with address";
// Keep this aligned with `union-bridge-client/config/base.toml`. Local and docker both point at
// the same Anvil deployment, so the CLI can rely on this fixed StreamManager address.
const LOCAL_STREAM_MANAGER_ADDRESS: &str = "0x0165878A594ca255338adfa4d48449f69242Eb8F";
const WEI_PER_RBTC: u64 = 1_000_000_000_000_000_000;
const WEI_PER_SAT: u64 = 10_000_000_000;
// Fixed local/dev gas headroom added on top of the pegout amount for user wallets.
const LOCAL_USER_RSK_GAS_BUFFER_WEI: u64 = 30_000_000_000_000_000;

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
        Environment::Docker => collect_local_signers(MEMBER_LOG_MARKER)?,
        Environment::Remote(_) => {
            let hosts = env.hosts()?;
            let ssh_user = env.remote_ssh_user()?;
            collect_remote_member_addresses(&hosts, &ssh_user)?
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
pub async fn handle_operator_funding(
    env: Environment,
    stream_id: u64,
    stream_manager_address: Option<&str>,
    roles: Option<&str>,
) -> Result<()> {
    match env {
        Environment::Local => {
            fund_local(stream_id)?;
        }
        Environment::Docker => {
            fund_local_docker(stream_id)?;
        }
        Environment::Remote(_) => {
            print_instructions(&env, stream_id, stream_manager_address, roles)?
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
        Environment::Docker => collect_user_rsk_addresses_from_local_docker(false)?,
        Environment::Remote(_) => collect_user_rsk_addresses_from_remote(&env, false)?,
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
                println!(
                    "Fund with `cast` using a key you control. Replace <PRIVATE_KEY> locally:"
                );
                for (_, address) in &user_addresses {
                    println!(
                        "  cast send {} --value <VARIABLE_AMOUNT_PER_STREAM> --private-key <PRIVATE_KEY> --rpc-url {}",
                        address, rpc_url
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
pub fn get_user_rsk_address(env: &Environment, first_only: bool) -> Result<Option<String>> {
    let addresses = match env {
        Environment::Local => collect_user_rsk_addresses_from_cargo_logs(first_only)?,
        Environment::Docker => collect_user_rsk_addresses_from_local_docker(first_only)?,
        Environment::Remote(_) => collect_user_rsk_addresses_from_remote(env, first_only)?,
    };
    Ok(addresses.into_iter().next().map(|(_, addr)| addr))
}

fn collect_user_rsk_addresses_from_cargo_logs(first_only: bool) -> Result<Vec<(String, String)>> {
    let logs_dir = cargo_logs_dir()?;
    let mut addresses = Vec::new();

    let all_ids = operator_ids();
    let ids: &[u8] = if first_only { &[1] } else { &all_ids };
    for operator_id in ids {
        let log_paths = local_log_paths(&logs_dir, "user-api", *operator_id)?;
        if log_paths.is_empty() {
            continue;
        }

        for log_path in log_paths {
            let contents = fs::read_to_string(&log_path)
                .with_context(|| format!("failed to read {}", log_path.display()))?;

            if let Some(address) = extract_user_rsk_address(&contents) {
                addresses.push((format!("user-api-{}", operator_id), address));
                break;
            }
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
    env: &Environment,
    first_only: bool,
) -> Result<Vec<(String, String)>> {
    let all_hosts = env.hosts()?;
    let hosts: Vec<&String> = if first_only {
        all_hosts.first().into_iter().collect()
    } else {
        all_hosts.iter().collect()
    };
    let mut addresses = Vec::new();

    for host in hosts {
        let target = format!("{}@{}", env.remote_ssh_user()?, host);

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

fn fund_local(stream_id: u64) -> Result<()> {
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
    let user_signers = collect_local_signers_from_logs(USER_LOG_MARKER)?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure all required operator services are running and have emitted signer addresses.",
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

fn fund_local_docker(stream_id: u64) -> Result<()> {
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
    let user_signers = collect_local_signers(USER_LOG_MARKER)?;
    let unique_users = unique_addresses(&user_signers);
    if unique_users.len() < expected {
        bail!(
            "expected {} user RSK address(es) but found {}. ensure all required operator stacks are running and have emitted signer addresses.",
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

fn print_instructions(
    env: &Environment,
    stream_id: u64,
    stream_manager_address: Option<&str>,
    roles: Option<&str>,
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

    println!("[docker-fund] gathering operator wallets from {} hosts", env_name);
    let ssh_user = env.remote_ssh_user()?;
    let signers = collect_remote_member_addresses(&hosts, &ssh_user)?;
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

    println!("Fund with `cast` using a key you control. Replace <PRIVATE_KEY> locally:");
    for (index, address) in unique.into_iter().enumerate() {
        let required_balance = required_operator_rsk_balance(
            &rpc_url,
            stream_manager_address,
            stream_id,
            roles[index],
        )?;
        println!(
            "  cast send {} --value {} --private-key <PRIVATE_KEY> --rpc-url {}",
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
    if index % 2 == 0 {
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
    let (operator_count, prover_count) = operator_and_prover_counts();
    let amount_in_wei = derive_stream_funding_profile(
        stream_id,
        true,
        COMMITTEE_PACKET_SIZE,
        operator_count,
        prover_count,
    )
    .map(|profile| U256::from(profile.denomination) * U256::from(WEI_PER_SAT))
    .ok_or_else(|| anyhow!("invalid stream id {} (expected 0-4)", stream_id))?;

    Ok(amount_in_wei + U256::from(LOCAL_USER_RSK_GAS_BUFFER_WEI))
}

fn collect_local_signers_from_logs(marker: &str) -> Result<Vec<(String, String)>> {
    let logs_dir = cargo_logs_dir()?;
    let mut signers = Vec::new();

    let log_type = if marker == MEMBER_LOG_MARKER { "coordinator" } else { "user-api" };

    for operator_id in operator_ids() {
        let log_paths = local_log_paths(&logs_dir, log_type, operator_id)?;
        if log_paths.is_empty() {
            bail!(
                "expected {} log for operator {} under {} but none exist. ensure the services have been started via `cargo run -- run`.",
                log_type,
                operator_id,
                logs_dir.display()
            );
        }

        let mut addresses = Vec::new();
        for log_path in &log_paths {
            let contents = fs::read_to_string(log_path)
                .with_context(|| format!("failed to read {}", log_path.display()))?;
            addresses.extend(extract_signer_addresses(&contents, marker));
            if !addresses.is_empty() {
                break;
            }
        }

        if addresses.is_empty() {
            let searched_paths = log_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "no {} signer addresses found in [{}]. wait for the {} to emit the log line and try again.",
                log_type,
                searched_paths,
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

fn local_log_paths(logs_dir: &Path, log_type: &str, operator_id: u8) -> Result<Vec<PathBuf>> {
    let current_name = format!("{log_type}-{operator_id}.log");
    let rotated_prefix = format!("{log_type}-{operator_id}.");
    let mut rotated = Vec::new();

    for entry in
        fs::read_dir(logs_dir).with_context(|| format!("failed to read {}", logs_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", logs_dir.display()))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        if file_name == current_name {
            continue;
        }

        if !file_name.starts_with(&rotated_prefix) || !file_name.ends_with(".log") {
            continue;
        }

        let suffix = &file_name[rotated_prefix.len()..file_name.len() - ".log".len()];
        let Ok(index) = suffix.parse::<u32>() else {
            continue;
        };
        rotated.push((index, entry.path()));
    }

    rotated.sort_by_key(|(index, _)| *index);

    let mut paths = Vec::new();
    let current_path = logs_dir.join(&current_name);
    if current_path.exists() {
        paths.push(current_path);
    }
    paths.extend(rotated.into_iter().map(|(_, path)| path));

    Ok(paths)
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

fn collect_remote_member_addresses(
    hosts: &[String],
    ssh_user: &str,
) -> Result<Vec<(String, String)>> {
    let mut signers = Vec::new();
    for host in hosts {
        let target = format!("{}@{}", ssh_user, host);

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

fn required_operator_rsk_balance(
    rpc_url: &str,
    stream_manager_address: &str,
    stream_id: u64,
    role: CommitteeFundingRole,
) -> Result<U256> {
    let min_deposit = fetch_stream_min_deposit(rpc_url, stream_manager_address, stream_id, role)?;
    let operator_count = u64::try_from(operator_ids().len()).expect("operator count fits in u64");
    Ok(required_member_rsk_balance(min_deposit, COMMITTEE_PACKET_SIZE, operator_count))
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
