use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

use crate::constants::{
    operator_and_prover_counts, operator_ids, COMMITTEE_PACKET_SIZE, ONE_OPERATOR_COMPOSE_PROJECT,
};
use crate::environments::*;
use crate::utils::command_to_string;
use op_funding::derive_stream_funding_profile;

const LOG_MARKER: &str = "Received BitVMX Funding Address:";

pub async fn handle_bitcoin_funding(
    environment: Environment,
    stream_id: u64,
    execute: bool,
    amount_override: Option<u64>,
) -> Result<()> {
    if execute && environment.is_remote() {
        bail!("--execute flag is only supported for local environments (`local`/`docker`). For remote environments, please run the wallet commands manually.");
    }

    let (operator_count, prover_count) = operator_and_prover_counts();
    let funding_profile = derive_stream_funding_profile(
        stream_id,
        matches!(environment, Environment::Local | Environment::Docker),
        COMMITTEE_PACKET_SIZE,
        operator_count,
        prover_count,
    )
    .with_context(|| format!("invalid stream id {} (expected 0-4)", stream_id))?;
    let amount = amount_override.unwrap_or(funding_profile.operator_fund_amount);

    let addresses = match &environment {
        Environment::Local => collect_local_addresses().await?,
        Environment::Docker => collect_local_docker_addresses().await?,
        Environment::Remote(_) => collect_remote_addresses(&environment).await?,
    };

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

async fn collect_local_addresses() -> Result<Vec<String>> {
    request_bitvmx_address_user_api(&Environment::Local).await?;

    let logs_dir = cargo_logs_dir()?;
    let mut addresses = Vec::new();

    for operator_id in operator_ids() {
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
    request_bitvmx_address_user_api(&Environment::Docker).await?;

    let projects: Vec<String> = operator_ids().iter().map(|id| format!("op_{}", id)).collect();
    collect_addresses_from_logs(projects, run_docker_compose_logs)
}

async fn collect_remote_addresses(env: &Environment) -> Result<Vec<String>> {
    request_bitvmx_address_user_api(env).await?;

    let hosts = env.hosts()?;
    let ssh_user = env.remote_ssh_user()?;

    collect_addresses_from_logs(hosts, |host| {
        let target = format!("{}@{}", ssh_user, host);
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

async fn request_bitvmx_address_user_api(environment: &Environment) -> Result<()> {
    let endpoints = environment.user_api_endpoints()?;

    println!("Triggering BitVMX endpoints: {} ...", endpoints.join(", "));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build http client")?;

    for endpoint in &endpoints {
        let url = format!("http://{}/member/bitvmx-address", endpoint);

        let request = client.get(&url).build()?;

        println!("GET {}", url);

        let response =
            client.execute(request).await.with_context(|| format!("failed to fetch {}", url))?;

        if !response.status().is_success() {
            bail!("request to {} failed with status {}", url, response.status());
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
    let output = cmd.output().with_context(|| format!("failed to run {}", error_context))?;

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
    cmd.arg(target).args(["docker", "compose", "-p", project, "logs"]);

    let cmd_str = command_to_string(&cmd);
    println!("{}", cmd_str);

    run_command_get_stdout(cmd, &format!("`{}`", cmd_str))
}

fn extract_last_bitvmx_address(log_content: &str) -> Option<String> {
    log_content
        .lines()
        .filter_map(|line| {
            line.find(LOG_MARKER).map(|idx| {
                let after = &line[idx + LOG_MARKER.len()..];
                after
                    .split_whitespace()
                    .next()
                    .map(|raw| raw.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string())
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
