use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

use crate::constants::{ONE_OPERATOR_COMPOSE_PROJECT, OPERATOR_IDS, REMOTE_SSH_USER};
use crate::environments::*;
use crate::utils::{command_to_string, confirm_operation, request_to_string};

const FUND_AMOUNT: &str = "20002000";
const LOG_MARKER: &str = "Received BitVMX Funding Address:";

pub async fn handle_bitcoin_funding(environment: Environment, execute: bool) -> Result<()> {
    if execute && environment.is_remote() {
        bail!("--execute flag is only supported for local environments (local/local-docker). For remote environments, please run the wallet commands manually.");
    }

    let addresses = match environment {
        Environment::Local => collect_local_addresses().await?,
        Environment::LocalDocker => collect_local_docker_addresses().await?,
        Environment::Alphanet | Environment::Testnet => {
            collect_remote_addresses(environment).await?
        }
    };

    if addresses.is_empty() {
        bail!("no BitVMX funding addresses were discovered");
    }

    println!();

    if execute {
        println!("Executing wallet commands programmatically...");
        println!();
        execute_wallet_command(&addresses)?;
    } else {
        print_instructions(environment, &addresses);
    }

    Ok(())
}

async fn collect_local_addresses() -> Result<Vec<String>> {
    request_bitvmx_address_user_api(Environment::Local).await?;

    let logs_dir = cargo_logs_dir()?;
    let mut addresses = Vec::new();

    for operator_id in OPERATOR_IDS {
        let log_path = logs_dir.join(format!("coordinator-{}.log", operator_id));
        if !log_path.exists() {
            bail!(
                "expected coordinator log at {} but it does not exist. ensure the services have been started via `cargo run -- run`.",
                log_path.display()
            );
        }

        let contents = fs::read_to_string(&log_path)
            .with_context(|| format!("failed to read {}", log_path.display()))?;
        if let Some(address) = extract_last_bitvmx_address(&contents) {
            println!("cargo coordinator-{} -> {}", operator_id, address);
            addresses.push(address);
        } else {
            bail!(
                "no BitVMX funding address found in {}. wait for the coordinator to emit the log line and try again.",
                log_path.display()
            );
        }
    }

    Ok(addresses)
}

async fn collect_local_docker_addresses() -> Result<Vec<String>> {
    request_bitvmx_address_user_api(Environment::LocalDocker).await?;

    let projects: Vec<String> = OPERATOR_IDS.iter().map(|id| format!("op_{}", id)).collect();
    collect_addresses_from_logs(projects, |project| run_docker_compose_logs(project))
}

async fn collect_remote_addresses(env: Environment) -> Result<Vec<String>> {
    request_bitvmx_address_user_api(env).await?;

    let hosts = env.hosts();

    collect_addresses_from_logs(hosts, |host| {
        let target = format!("{}@{}", REMOTE_SSH_USER, host);
        run_ssh_docker_compose_logs(&target, ONE_OPERATOR_COMPOSE_PROJECT)
    })
}

fn collect_addresses_from_logs<T, I, F>(items: T, mut get_logs: F) -> Result<Vec<String>>
where
    T: IntoIterator<Item = I>,
    I: AsRef<str>,
    F: FnMut(&str) -> Result<String>,
{
    let mut addresses = Vec::new();
    for item in items {
        let item_ref = item.as_ref();
        let logs = get_logs(item_ref)?;
        if let Some(address) = extract_last_bitvmx_address(&logs) {
            println!("{} -> {}", item_ref, address);
            addresses.push(address);
        } else {
            bail!(
                "no BitVMX funding address found in docker logs for {}. ensure the stack is running and has emitted the log line.",
                item_ref
            );
        }
    }
    Ok(addresses)
}

async fn request_bitvmx_address_user_api(environment: Environment) -> Result<()> {
    let endpoints = environment.user_api_endpoints();

    println!("Triggering BitVMX endpoints: {} ...", endpoints.join(", "));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build http client")?;

    for endpoint in &endpoints {
        let url = format!("http://{}/member/bitvmx-address", endpoint);

        let request = client.get(&url).build()?;

        if environment.is_remote() {
            let description = request_to_string(&request);
            if !confirm_operation(&description)? {
                bail!("Operation cancelled by user");
            }
        } else {
            println!("GET {}", url);
        }

        let response = client
            .execute(request)
            .await
            .with_context(|| format!("failed to fetch {}", url))?;

        if !response.status().is_success() {
            bail!(
                "request to {} failed with status {}",
                url,
                response.status()
            );
        }

        let body = response
            .text()
            .await
            .with_context(|| format!("failed to read response body from {}", url))?;
        println!("{}", body.trim());
    }

    sleep(Duration::from_secs(10)).await;

    Ok(())
}

fn run_command_get_stdout(mut cmd: Command, error_context: &str) -> Result<String> {
    let output = cmd
        .output()
        .with_context(|| format!("failed to run {}", error_context))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{} failed with: {}", error_context, stderr.trim());
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("{} output is not valid utf-8", error_context))
}

fn run_docker_compose_logs(project: &str) -> Result<String> {
    let mut cmd = Command::new("docker");
    cmd.args(["compose", "-p", project, "logs"]);
    run_command_get_stdout(cmd, &format!("`docker compose -p {} logs`", project))
}

fn run_ssh_docker_compose_logs(target: &str, project: &str) -> Result<String> {
    let mut cmd = Command::new("ssh");
    cmd.arg(target)
        .args(["docker", "compose", "-p", project, "logs"]);

    let cmd_str = command_to_string(&cmd);

    if !confirm_operation(&cmd_str)? {
        bail!("Operation cancelled by user");
    }

    run_command_get_stdout(cmd, &format!("`{}`", cmd_str))
}

fn extract_last_bitvmx_address(log_content: &str) -> Option<String> {
    log_content
        .lines()
        .filter_map(|line| {
            line.find(LOG_MARKER).map(|idx| {
                let after = &line[idx + LOG_MARKER.len()..];
                after.split_whitespace().next().map(|raw| {
                    raw.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                        .to_string()
                })
            })
        })
        .flatten()
        .last()
}

fn cargo_logs_dir() -> Result<PathBuf> {
    let run_cli_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project_root = run_cli_dir
        .parent()
        .expect("failed to get logs dir")
        .parent()
        .ok_or_else(|| anyhow!("led to get logs dir"))?;
    Ok(project_root.join("logs"))
}

fn print_instructions(env: Environment, addresses: &[String]) {
    let joined = addresses.join(",");
    println!("Note: See the bitcoin-wallet README for how to start and use the CLI: ../cli/bitcoin-wallet/README.md\n");

    match env {
        Environment::Alphanet | Environment::Testnet => {
            println!(
                "Run the following command in your bitcoin-wallet or wallet tooling for {}:",
                env.get_name()
            );
            println!("  send_to_address {} {}\n", joined, FUND_AMOUNT);
        }
        Environment::LocalDocker | Environment::Local => {
            println!("Run the following commands in the bitcoin-wallet CLI (Regtest):");
            println!("1 =>    clear_db   (if you see a misaligned utxos error)");
            println!("2 =>    mine_utxo 900000000");
            println!("3 =>    send_to_address {} {}", joined, FUND_AMOUNT);
            println!("4 =>    mine_block");
        }
    }
}

fn execute_wallet_command(addresses: &[String]) -> Result<()> {
    let wallet_script = "./cli-bitcoin-wallet.sh";
    let joined = addresses.join(",");

    // just send to addresses - utxo mining and block mining handled externally
    let mut cmd = Command::new(wallet_script);
    cmd.arg("member")
        .arg("send_to_address")
        .arg(&joined)
        .arg(FUND_AMOUNT);

    println!(
        "Running: {} member send_to_address {} {}",
        wallet_script, joined, FUND_AMOUNT
    );

    let output = cmd
        .output()
        .context("failed to execute cli-bitcoin-wallet.sh")?;

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
