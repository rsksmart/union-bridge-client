//! union bridge client launcher and utility toolkit
//!
//! subcommands:
//! - `run`: orchestrates the four union bridge services for one or many clients.
//!   use `-n` for multi-client mode, `--id` for a single client,
//!   `--features` to pass cargo feature flags, and `--fresh` to wipe local state.
//! - `create-rootstock-wallets`: creates rootstock keystores for local multi-client
//!   deployments.
//! - `fund-ops-rootstock`: funds rootstock wallets for operator stacks.
//!   use `--env local-docker` (default) for local docker compose stacks, or
//!   `--env alphanet` for remote alphanet stacks.
//! - `fund-ops-bitcoin`: collects bitcoin funding addresses for operators.
//!   use `--env local` (default) for cargo-run coordinators, `--env local-docker`
//!   for local docker compose stacks, or `--env alphanet` for remote stacks.
//! - `create-pegin-tx`: requests a pegin address and prints bitcoin-wallet
//!   instructions. requires `--rsk-address/-a`, with optional stream and packet
//!   overrides via `--stream-amount/-s` and `--packet-number/-p`.
//! - `setup-committee`: applies operators to a stream. requires `--stream-id/-s`,
//!   defaults to the local environment, and accepts `--env` (`local` or `alphanet`)
//!   plus optional `--role` when targeting alphanet.
//!
//! quick examples:
//! ```bash
//! cargo run -- run -n 4 --fresh
//! cargo run -- create-rootstock-wallets
//! cargo run -- fund-ops-rootstock --env local-docker
//! cargo run -- fund-ops-bitcoin --env local
//! cargo run -- create-pegin-tx -a 0x1234...cdef -s 2_000_000 -p 7
//! cargo run -- setup-committee -s 1
//! ```

mod bitcoin_wallet;
mod committee;
mod config;
mod pegin;
mod rsk_wallet;

use crate::committee::CommitteeRole;
use crate::config::{Environment, OPERATOR_IDS};
use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgAction, Parser, Subcommand};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal;
use tokio::sync::broadcast;

#[derive(Debug, Parser, Clone)]
#[command(
    name = "run-cli",
    about = "Union Bridge Client Launcher and Wallet Setup"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand, Clone)]
enum Commands {
    /// Run Union Bridge client services
    Run {
        /// Number of clients to run (1-10). If provided, multi-client mode is used.
        #[arg(short = 'n', long = "num-clients")]
        num_clients: Option<u8>,

        /// Run a single client with the specified ID (1-10). Defaults to 1 if neither mode flag is passed.
        #[arg(short = 'i', long = "id")]
        client_id: Option<u8>,

        /// Optional features to pass to cargo (e.g. "anvil").
        #[arg(short = 'f', long = "features")]
        features: Option<String>,

        /// Start with clear databases (removes existing)
        #[arg(long = "fresh", action = ArgAction::SetTrue)]
        fresh: bool,

        /// Path to multiclient.env. Defaults to ./multiclient.env if it exists
        #[arg(long = "env-file")]
        env_file: Option<PathBuf>,
    },
    /// Request a pegin address and print bitcoin-wallet CLI instructions
    #[command(name = "create-pegin-tx")]
    CreatePeginTx {
        /// Rootstock deposit address
        #[arg(short = 'a', long = "rsk-address", value_name = "RSK_ADDRESS")]
        rsk_address: String,

        /// Amount (in satoshis) to stream into the pegin transaction
        #[arg(
            short = 's',
            long = "stream-amount",
            value_name = "STREAM_AMOUNT",
            default_value_t = 1_000_000u64
        )]
        stream_amount: u64,

        /// Packet number used when creating the pegin transaction
        #[arg(
            short = 'p',
            long = "packet-number",
            value_name = "PACKET_NUMBER",
            default_value_t = 0u64
        )]
        packet_number: u64,
    },
    /// Create Rootstock wallets for local multi-client deployment
    #[command(name = "create-rootstock-wallets")]
    CreateRootstockWallets,
    /// Collect BitVMX funding addresses for operators
    #[command(name = "fund-ops-bitcoin")]
    FundOperatorsBitcoin {
        /// Environment to target (local, local-docker, alphanet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::Local)]
        env: Environment,
    },
    /// Fund Rootstock wallets for operator stacks
    #[command(name = "fund-ops-rootstock")]
    FundOperatorsRootstock {
        /// Environment to target (local-docker, alphanet)
        #[arg(long = "env", short = 'e', value_enum, default_value_t = Environment::LocalDocker)]
        env: Environment,
    },
    /// Apply operators to a stream for committee setup
    #[command(name = "setup-committee")]
    SetupCommittee {
        /// Stream identifier to configure
        #[arg(short = 's', long = "stream-id", value_name = "STREAM_ID")]
        stream_id: u64,

        /// Target environment (`local` or `alphanet`)
        #[arg(long = "env", value_enum, default_value_t = Environment::Local)]
        env: Environment,

        /// Operator role when applying on alphanet
        #[arg(long = "role", value_enum)]
        role: Option<CommitteeRole>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Service {
    BlockIndexer,
    LogIndexer,
    UserApi,
    Coordinator,
}

impl Service {
    fn name(&self) -> &'static str {
        match self {
            Service::BlockIndexer => "block-indexer",
            Service::LogIndexer => "log-indexer",
            Service::UserApi => "user-api",
            Service::Coordinator => "coordinator",
        }
    }
}

const UNION_CLIENT_SERVICES: [Service; 4] = [
    Service::BlockIndexer,
    Service::LogIndexer,
    Service::UserApi,
    Service::Coordinator,
];

#[derive(Debug, Clone)]
struct ManagedService {
    service: String,
    pid: u32,
    child: Arc<Mutex<Child>>,
}

#[derive(Debug, Clone)]
struct ManagedClient {
    client_id: String,
    services: Vec<ManagedService>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreatePeginTx {
            rsk_address,
            stream_amount,
            packet_number,
        } => {
            pegin::create_pegin_tx(rsk_address, stream_amount, packet_number).await?;
        }
        Commands::CreateRootstockWallets => {
            let base_storage_path = std::env::var("BASE_STORAGE_PATH").ok();
            rsk_wallet::handle_wallet_creation(
                OPERATOR_IDS.len() as u8,
                base_storage_path.as_deref(),
            )?;
        }
        Commands::FundOperatorsBitcoin { env } => {
            bitcoin_wallet::handle_bitcoin_funding(env).await?;
        }
        Commands::FundOperatorsRootstock { env } => {
            rsk_wallet::handle_operator_funding(env).await?;
        }
        Commands::Run {
            num_clients,
            client_id,
            features,
            fresh,
            env_file,
        } => {
            let base_storage_path = std::env::var("BASE_STORAGE_PATH").context(
                "BASE_STORAGE_PATH environment variable is required (e.g., export BASE_STORAGE_PATH=/Users/username)",
            )?;
            let run_config = RunConfig {
                num_clients,
                client_id,
                features,
                fresh,
                env_file,
            };
            run_clients(run_config, &base_storage_path).await?;
        }
        Commands::SetupCommittee {
            stream_id,
            env,
            role,
        } => {
            committee::run_committee_setup(stream_id, env, role).await?;
        }
    }

    Ok(())
}

#[derive(Clone)]
struct RunConfig {
    num_clients: Option<u8>,
    client_id: Option<u8>,
    features: Option<String>,
    fresh: bool,
    env_file: Option<PathBuf>,
}

async fn run_clients(config: RunConfig, base_storage_path: &str) -> Result<()> {
    if config.num_clients.is_some() && config.client_id.is_some() {
        return Err(anyhow!("Cannot specify both -n and --id at the same time"));
    }

    if config.fresh {
        fresh_cleanup(base_storage_path)?;
    }

    let env_file = resolve_env_file(config.env_file.as_deref());
    let env_map = if let Some(path) = env_file {
        load_env_file(&path).with_context(|| format!("Failed to parse {}", path.display()))?
    } else {
        HashMap::new()
    };

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    // Keep all clients and join handles for monitors
    let mut clients: Vec<ManagedClient> = Vec::new();

    let total = config.num_clients.unwrap_or(1);
    for id in 1..=total {
        let envs = build_env_for_client(id, &env_map, base_storage_path)?;
        let client_id = format!("client-{}", id);

        println!("============================================================================");
        println!("Launching client {} with env {:?}...", client_id, envs);
        println!("============================================================================");

        match launch_client_services(&config, envs, &client_id, &shutdown_tx) {
            Ok(services) => {
                let client = ManagedClient {
                    client_id: client_id.to_string(),
                    services,
                };
                clients.push(client);
            }
            Err(_) => {
                println!("Failed to launch all services for {client_id}");
                break;
            }
        }
    }

    // Ctrl+C handler
    let ctrlc_tx = shutdown_tx.clone();
    let ctrlc_handle = tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        let _ = ctrlc_tx.send(());
    });

    // Wait for one of: a) any monitor triggers shutdown, b) Ctrl+C
    tokio::select! {
        _ = shutdown_rx.recv() => {
            // shutdown requested
        }
    }

    println!("Shutdown requested");

    // Teardown everything
    teardown_all(clients).await;

    println!("All clients shut down");

    // Stop the Ctrl+C handler to avoid keeping the runtime alive
    ctrlc_handle.abort();

    println!("Done");

    Ok(())
}

pub(crate) fn validate_1_10(value: u8, name: &str) -> Result<()> {
    if !(1..=10).contains(&value) {
        return Err(anyhow!("{} must be between 1 and 10", name));
    }
    Ok(())
}

fn resolve_env_file(opt: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = opt {
        return Some(p.to_path_buf());
    }
    let default = PathBuf::from("./multiclient.env");
    if default.exists() {
        Some(default)
    } else {
        None
    }
}

fn load_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = fs::read_to_string(path)?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let mut val = v.trim().to_string();
            // Strip optional quotes
            if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                val = val[1..val.len() - 1].to_string();
            }
            map.insert(key, val);
        }
    }
    Ok(map)
}

fn build_env_for_client(
    id: u8,
    env_map: &HashMap<String, String>,
    base_storage_path: &str,
) -> Result<Vec<(String, String)>> {
    validate_1_10(id, "CLIENT_ID")?;
    let get = |key: String| -> Result<String> {
        env_map
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow!("Missing {} in multiclient.env", key))
    };

    let storage_rel = get(format!("STORAGE_PATH_{}", id))?;
    let storage_path = format!(
        "{}/.union_bridge/database/{}",
        base_storage_path, storage_rel
    );

    let key_store_base = get(format!("KEY_STORE_PATH_{}", id))?;
    let (key_store_member_name, key_store_user_name) = derive_key_store_names(&key_store_base);
    let key_store_member_path = format!(
        "{}/.union_bridge/keystore/{}",
        base_storage_path, key_store_member_name
    );
    let key_store_user_path = format!(
        "{}/.union_bridge/keystore/{}",
        base_storage_path, key_store_user_name
    );

    let envs: Vec<(String, String)> = vec![
        (
            "UB__BLOCK_NOTIFIER__BROKER_PORT".into(),
            get(format!("BLOCK_NOTIFIER_BROKER_PORT_{}", id))?,
        ),
        (
            "UB__LOG_NOTIFIER__BROKER_PORT".into(),
            get(format!("LOG_NOTIFIER_BROKER_PORT_{}", id))?,
        ),
        (
            "UB__BLOCK_BROKER__PORT".into(),
            get(format!("BLOCK_BROKER_PORT_{}", id))?,
        ),
        (
            "UB__LOG_BROKER__PORT".into(),
            get(format!("LOG_BROKER_PORT_{}", id))?,
        ),
        (
            "UB__USER_BROKER__PORT".into(),
            get(format!("USER_BROKER_PORT_{}", id))?,
        ),
        (
            "UB__BROKER_CLIENT_ID".into(),
            get(format!("BROKER_CLIENT_ID_{}", id))?,
        ),
        ("UB__INDEXER__STORAGE__PATH".into(), storage_path.clone()),
        ("UB__STORAGE_PATH".into(), storage_path),
        ("UB__KEY_STORE__MEMBER_PATH".into(), key_store_member_path),
        ("UB__KEY_STORE__USER_PATH".into(), key_store_user_path),
        ("UB__SERVER__URL".into(), get(format!("SERVER_URL_{}", id))?),
        (
            "UB__COORDINATOR_BROKER_CLIENT_ID".into(),
            get(format!("COORDINATOR_BROKER_CLIENT_ID_{}", id))?,
        ),
        (
            "UB__BROKER_SERVER_PORT".into(),
            get(format!("BROKER_SERVER_PORT_{}", id))?,
        ),
        (
            "UB__HTTP_SERVER_PORT".into(),
            get(format!("HTTP_SERVER_PORT_{}", id))?,
        ),
        (
            "UB__BITVMX_BROKER__PORT".into(),
            get(format!("BITVMX_BROKER_PORT_{}", id))?,
        ),
        ("CLIENT_ID".into(), id.to_string()),
    ];

    Ok(envs)
}

fn derive_key_store_names(raw: &str) -> (String, String) {
    let base = raw
        .strip_suffix("-member")
        .or_else(|| raw.strip_suffix("-user"))
        .unwrap_or(raw)
        .to_string();

    (format!("{base}-member"), format!("{base}-user"))
}

fn fresh_cleanup(base_storage_path: &str) -> Result<()> {
    let union_client_db_dir = format!("{}/.union_bridge/database/multi-client", base_storage_path);
    let bitvmx_db_dir = "/tmp/regtest";
    let bitvmx_p2p_dir = "/tmp/broker_p2p";

    // Remove Union Bridge database directory
    if Path::new(&union_client_db_dir).exists() {
        fs::remove_dir_all(&union_client_db_dir)
            .with_context(|| format!("Failed to remove {}", union_client_db_dir))?;
    }

    // Remove BitVMX db dir
    if Path::new(bitvmx_db_dir).exists() {
        fs::remove_dir_all(bitvmx_db_dir)
            .with_context(|| format!("Failed to remove {}", bitvmx_db_dir))?;
    }

    // Remove BitVMX p2p dir
    if Path::new(bitvmx_p2p_dir).exists() {
        fs::remove_dir_all(bitvmx_p2p_dir)
            .with_context(|| format!("Failed to remove {}", bitvmx_p2p_dir))?;
    }

    Ok(())
}

fn cargo_args_for_service(config: &RunConfig, svc: &Service) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into(), "--bin".into(), svc.name().into()];
    if let Some(f) = &config.features {
        args.push("--features".into());
        args.push(f.clone());
    }
    args.push("--".into());
    args.push("--env".into());
    args.push("local".into());
    args
}

fn launch_client_services(
    config: &RunConfig,
    envs: Vec<(String, String)>,
    client_id: &str,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<Vec<ManagedService>> {
    let mut services: Vec<ManagedService> = Vec::new();

    for svc in UNION_CLIENT_SERVICES {
        println!("Launching {} for {}", svc.name(), client_id);

        // Coordinator depends loosely on others, wait a little before starting it
        if svc == Service::Coordinator {
            std::thread::sleep(Duration::from_secs(2));
        }

        let mut cmd = Command::new("cargo");
        let args = cargo_args_for_service(config, &svc);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(0); // create new process group to avoid receiving parent's SIGINT

        let child = cmd
            .envs(envs.clone())
            .spawn()
            .with_context(|| format!("Failed to start {} for {}", svc.name(), client_id))?;
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        let mgd_child = ManagedService {
            service: svc.name().to_string(),
            pid,
            child,
        };
        services.push(mgd_child);
    }

    // Quick small delay to see if any exited immediately
    std::thread::sleep(Duration::from_secs(2));

    // Verify none exited immediately
    for ms in services.iter() {
        let name = format!("{}:{}", client_id, ms.service);
        let mut guard = ms
            .child
            .lock()
            .expect(format!("Failed to lock {}", name).as_str());
        if let Ok(Some(status)) = guard.try_wait() {
            println!("ERROR: {} exited immediately with status {}", name, status);
            let _ = shutdown_tx.send(());
            bail!("Failed to launch all services for {}", client_id);
        }
    }

    // Spawn monitors
    for ms in services.iter() {
        let tx = shutdown_tx.clone();
        let monitor_name = format!("{}:{}", client_id, ms.service);
        let child_for_monitor = ms.child.clone();
        tokio::spawn(async move {
            // Blocking wait in a blocking thread so as not to block runtime
            let status = tokio::task::spawn_blocking(move || {
                let mut g = child_for_monitor.lock().expect("Failed to lock");
                g.wait()
            })
            .await
            .ok()
            .and_then(|r| r.ok());
            if let Some(status) = status {
                eprintln!(
                    "Process {} exited with status {}. Initiating shutdown...",
                    monitor_name, status
                );
            } else {
                eprintln!(
                    "Process {} wait failed. Initiating shutdown...",
                    monitor_name
                );
            }
            let _ = tx.send(());
        });
    }

    Ok(services)
}

async fn teardown_all(clients: Vec<ManagedClient>) {
    // teardown all clients simultaneously, but within each client, stop services with proper ordering
    let mut handles = Vec::new();

    for client in clients {
        let handle = tokio::spawn(async move {
            println!("Tearing down {}", client.client_id);

            // shutdown coordinator first and wait for it to exit completely
            // this ensures it can properly unsubscribe from brokers before they shut down
            if let Some(coordinator) = client
                .services
                .iter()
                .rev()
                .find(|s| s.service == "coordinator")
            {
                let pid = Pid::from_raw(coordinator.pid as i32);
                let _ = kill(pid, Signal::SIGTERM);

                // wait for coordinator to exit (up to 10 seconds)
                let start = std::time::Instant::now();
                let coordinator_timeout = Duration::from_secs(10);
                loop {
                    if let Ok(mut guard) = coordinator.child.lock() {
                        if let Ok(Some(_)) = guard.try_wait() {
                            // coordinator has exited
                            break;
                        }
                    }
                    if start.elapsed() >= coordinator_timeout {
                        // timeout, force kill
                        let _ = kill(pid, Signal::SIGKILL);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }

                // additional delay after coordinator exits to ensure all cleanup operations complete
                // this allows network messages (broker unsubscription) to be sent and acknowledged
                // before indexers start shutting down their broker servers
                tokio::time::sleep(Duration::from_secs(4)).await;
            }

            // now shutdown remaining services (in reverse order, skipping coordinator)
            for svc in client.services.iter().rev() {
                if svc.service == "coordinator" {
                    continue; // already handled
                }
                let pid = Pid::from_raw(svc.pid as i32);
                let _ = kill(pid, Signal::SIGTERM);
                // small delay between signals to stagger shutdown
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // poll for up to 3 seconds to allow graceful exit of remaining services
            let start = std::time::Instant::now();
            let graceful_timeout = Duration::from_secs(3);
            loop {
                let mut remaining = 0usize;
                for svc in client.services.iter() {
                    if svc.service == "coordinator" {
                        continue; // already exited
                    }
                    if let Ok(mut guard) = svc.child.lock() {
                        if let Ok(None) = guard.try_wait() {
                            remaining += 1;
                        }
                    } else {
                        remaining += 1;
                    }
                }
                if remaining == 0 {
                    break;
                }
                if start.elapsed() >= graceful_timeout {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            // SIGKILL any stragglers
            for svc in client.services.iter().rev() {
                let pid = Pid::from_raw(svc.pid as i32);
                if let Ok(mut guard) = svc.child.lock() {
                    if let Ok(None) = guard.try_wait() {
                        let _ = kill(pid, Signal::SIGKILL);
                    }
                } else {
                    let _ = kill(pid, Signal::SIGKILL);
                }
            }

            // brief final wait for monitors to reap
            tokio::time::sleep(Duration::from_millis(300)).await;
        });
        handles.push(handle);
    }

    // wait for all client teardowns to complete
    for h in handles {
        let _ = h.await;
    }
}
