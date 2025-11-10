use anyhow::{anyhow, bail, Context, Result};
use key_manager::key_manager::KeyManager;
use rpassword::prompt_password;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::constants::{
    AWS_SSH_USER, LOCAL_ANVIL_ADDRESS, ONE_OPERATOR_COMPOSE_PROJECT, OPERATOR_IDS,
};
use crate::environments::*;
use crate::validate_1_4;

const LOG_MARKER: &str = "Got member signer with address";

/// handles creating local rootstock wallets for multi-client deployments
pub fn handle_wallet_creation(num_wallets: u8, base_storage_path: Option<&str>) -> Result<()> {
    let base = require_base_storage_path(base_storage_path)?;
    validate_1_4(num_wallets, "num-wallets")?;

    setup_wallets_create(num_wallets, base)?;
    print_wallet_summary("create", num_wallets);

    Ok(())
}

/// handles funding rootstock wallets for operator stacks
pub async fn handle_operator_funding(env: Environment) -> Result<()> {
    match env {
        Environment::LocalDocker => {
            fund_local_docker()?;
        }
        Environment::Alphanet => {
            print_instructions(Environment::Alphanet)?;
        }
        Environment::Testnet => {
            print_instructions(Environment::Testnet)?;
        }
        Environment::Local => {
            bail!("Environment::Local is not supported for funding rootstock operators. Use LocalDocker, Alphanet, or Testnet.");
        }
    }
    Ok(())
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
    println!(
        "total wallets: {} (member + user per client)",
        num_wallets * 2
    );
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

    println!(
        "[wallet-setup] creating new key at {}...",
        keystore_path.display()
    );

    // create directory if it doesn't exist
    fs::create_dir_all(keystore_path)
        .with_context(|| format!("failed to create directory {}", keystore_path.display()))?;

    // generate key using KeyManager directly
    let (generated_path, _public_key, _address) = KeyManager::generate_key(keystore_path, password)
        .context("failed to generate key with KeyManager")?;

    // rename the generated key to the desired filename
    fs::rename(&generated_path, &full_keystore_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            generated_path,
            full_keystore_path.display()
        )
    })?;

    println!(
        "[wallet-setup] key created successfully at {}",
        full_keystore_path.display()
    );

    Ok(())
}

fn setup_wallets_create(num_wallets: u8, base_storage_path: &str) -> Result<()> {
    let password = std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable is required")?;

    let keystore_base_path = PathBuf::from(base_storage_path)
        .join(".union_bridge")
        .join("keystore");

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

fn fund_local_docker() -> Result<()> {
    println!("[docker-fund] funding operator wallets via local anvil");
    let signers = collect_local_signers()?;
    let unique = unique_addresses(&signers);
    let expected = OPERATOR_IDS.len();
    if unique.len() < expected {
        bail!(
            "expected {} RSK address(es) but found {}. ensure all required operator stacks are running and have emitted signer addresses.",
            expected,
            unique.len()
        );
    }

    for (project, address) in signers {
        println!("Processing {}", project);
        println!("  Funding RSK address: {}", address);
        run_cast_send_local(&address)?;
    }

    println!("Done. Funded operator RSK addresses on local Anvil.");
    Ok(())
}

fn print_instructions(env: Environment) -> Result<()> {
    let env_name = env.get_name();

    let hosts = env.hosts();
    let rpc_url = env.rpc_url();

    println!(
        "[docker-fund] gathering operator wallets from {} hosts",
        env_name
    );
    let signers = collect_aws_signers(&hosts)?;
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

fn collect_local_signers() -> Result<Vec<(String, String)>> {
    let mut signers = Vec::new();
    for id in OPERATOR_IDS {
        let project = format!("op_{}", id);
        eprintln!("[docker-fund] running: docker compose -p {} logs", &project);
        let output = Command::new("docker")
            .args(["compose", "-p", &project, "logs"])
            .output()
            .with_context(|| format!("failed to run `docker compose -p {} logs`", &project))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "`docker compose -p {} logs` failed with: {}",
                &project,
                stderr.trim()
            );
        }
        let stdout = String::from_utf8(output.stdout)
            .context("docker compose logs output is not valid utf-8")?;
        let mut addresses = extract_signer_addresses(&stdout);
        if addresses.is_empty() {
            println!(
                "[docker-fund] no signer addresses found for project {}",
                project
            );
        } else {
            for address in addresses.drain(..) {
                signers.push((project.to_string(), address));
            }
        }
    }

    Ok(signers)
}

fn collect_aws_signers(hosts: &[String]) -> Result<Vec<(String, String)>> {
    let mut signers = Vec::new();
    for host in hosts {
        let target = format!("{}@{}", AWS_SSH_USER, host);
        eprintln!(
            "[docker-fund] running: ssh {} docker compose -p {} logs",
            target, ONE_OPERATOR_COMPOSE_PROJECT
        );
        let output = Command::new("ssh")
            .arg(&target)
            .args([
                "docker",
                "compose",
                "-p",
                ONE_OPERATOR_COMPOSE_PROJECT,
                "logs",
            ])
            .output()
            .with_context(|| {
                format!(
                    "failed to run `ssh {} docker compose -p {} logs`",
                    target, ONE_OPERATOR_COMPOSE_PROJECT
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "`ssh {} docker compose -p {} logs` failed with: {}",
                target,
                ONE_OPERATOR_COMPOSE_PROJECT,
                stderr.trim()
            );
        }
        let stdout = String::from_utf8(output.stdout).context("ssh output is not valid utf-8")?;
        let mut addresses = extract_signer_addresses(&stdout);
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

fn extract_signer_addresses(log_content: &str) -> Vec<String> {
    let mut unique = HashSet::new();
    for line in log_content.lines() {
        if let Some(idx) = line.find(LOG_MARKER) {
            let after_marker = &line[idx + LOG_MARKER.len()..];
            if let Some(candidate) = after_marker
                .split_whitespace()
                .find(|token| token.starts_with("0x"))
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
        .arg("1ether")
        .arg("--unlocked")
        .output()
        .context("failed to execute cast send")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cast send failed for {}: {}", address, stderr.trim());
    }

    Ok(())
}
