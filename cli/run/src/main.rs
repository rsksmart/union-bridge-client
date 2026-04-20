//! union bridge local client launcher
//!
//! orchestrates multiple union bridge client instances for local development and testing.
//! each client runs four services: block-indexer, log-indexer, coordinator, and user-api.
//!
//! ## usage
//!
//! run all 4 clients (simulates a 4-operator committee):
//! ```bash
//! cargo run
//! ```
//!
//! run a single client by id (1-4):
//! ```bash
//! cargo run -- --id 1
//! ```
//!
//! start with fresh databases (wipes all existing state):
//! ```bash
//! cargo run -- --fresh
//! ```
//!
//! choose BitVMX identity source:
//! ```bash
//! cargo run -- --bitvmx-mode docker   # default, containers use .union_bridge/op_N/bitvmx/keys/services.pubkey_hash
//! cargo run -- --bitvmx-mode repo     # running from cloned repo, ignores UB__COORDINATOR__BITVMX__PUBKEY_HASH_FILE_N override in [local-committee.env](../config/env_overrides/local-committee.env) and uses config/base.toml hash (matches bitvmx repo value)
//! ```
//!
//! pass custom cargo features:
//! ```bash
//! cargo run -- --features anvil
//! ```
//!
//! ## what it does
//!
//! for each client (1-4):
//! 1. creates separate databases for block-indexer, log-indexer, and coordinator
//! 2. launches block-indexer (monitors bitcoin blockchain)
//! 3. launches log-indexer (monitors rootstock smart contract events)
//! 4. launches coordinator (orchestrates bitvmx protocol operations)
//! 5. launches user-api (provides http api for pegin/pegout requests)
//!
//! each service reads from `config/base.toml` and profile-specific overrides
//! in `config/*.toml`.
//!
//! ## process management
//!
//! - graceful shutdown: press ctrl+c to stop all services
//! - all child processes are properly terminated on exit
//! - databases persist between runs unless `--fresh` is specified
//!
//! ## default ports (for client 1)
//!
//! - block-indexer: 50001
//! - log-indexer: 60001  
//! - coordinator: 40001
//! - user-api: 30001
//!
//! subsequent clients use incremental ports (e.g., client 2 uses 50002, 60002, 40002, 30002)

use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgAction, Parser, ValueEnum};
use key_manager::key_manager::KeyManager;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::fs;
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tokio::runtime::Runtime;
use tokio::signal;
use tokio::sync::broadcast;

// global state for panic handling
static ACTIVE_CLIENTS: Mutex<Option<Vec<ManagedClient>>> = Mutex::new(None);

#[derive(Debug, Parser, Clone)]
#[command(name = "run", about = "Union Bridge Local Client Launcher")]
struct Cli {
    /// Run a single client with the specified ID (1-4). If not provided, runs 4 clients.
    #[arg(short = 'i', long = "id")]
    client_id: Option<u8>,

    /// Optional features to pass to cargo (e.g. "anvil").
    #[arg(short = 'f', long = "features")]
    features: Option<String>,

    /// Start with clear databases (removes existing)
    #[arg(long = "fresh", action = ArgAction::SetTrue)]
    fresh: bool,

    /// Kill all existing running services and exit
    #[arg(long = "kill", action = ArgAction::SetTrue)]
    kill: bool,

    /// Source of BitVMX identity used by coordinator in local runs.
    #[arg(long = "bitvmx-mode", value_enum, default_value_t = BitvmxMode::Docker)]
    bitvmx_mode: BitvmxMode,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum BitvmxMode {
    Repo,
    Docker,
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

const UNION_CLIENT_SERVICES: [Service; 4] =
    [Service::BlockIndexer, Service::LogIndexer, Service::UserApi, Service::Coordinator];

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
    // install panic hook to ensure cleanup on panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        eprintln!("PANIC detected: {}", panic_info);
        eprintln!("Attempting to shut down all services...");

        // try to get clients from global state
        if let Ok(mut guard) = ACTIVE_CLIENTS.lock() {
            if let Some(clients) = guard.take() {
                eprintln!("Found {} client(s) to shut down", clients.len());
                // create a runtime for async cleanup since we're in a panic context
                if let Ok(rt) = Runtime::new() {
                    rt.block_on(teardown_all(clients));
                    eprintln!("Emergency shutdown complete");
                }
            }
        }

        // call the default panic handler
        default_panic(panic_info);
    }));

    let cli = Cli::parse();

    // handle kill command - exit early after killing services
    if cli.kill {
        detect_and_kill_existing_services()?;
        println!("Kill command ran, exiting...");
        return Ok(());
    }

    // validate BASE_STORAGE_PATH is set
    std::env::var("BASE_STORAGE_PATH").context(
        "BASE_STORAGE_PATH environment variable is required (e.g., export BASE_STORAGE_PATH=/Users/username)",
    )?;

    let run_config = RunConfig {
        client_id: cli.client_id,
        features: cli.features,
        fresh: cli.fresh,
        bitvmx_mode: cli.bitvmx_mode,
    };

    let result = run_clients(run_config).await;

    // cleanup global state on normal exit
    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(mut guard) = ACTIVE_CLIENTS.lock() {
            let _ = guard.take();
        }
    })
    .await;

    result
}

#[derive(Clone)]
struct RunConfig {
    client_id: Option<u8>,
    features: Option<String>,
    fresh: bool,
    bitvmx_mode: BitvmxMode,
}

async fn run_clients(config: RunConfig) -> Result<()> {
    if config.client_id.is_none() {
        // detect and kill any running services before starting
        detect_and_kill_existing_services()?;
    }

    if config.fresh {
        fresh_cleanup(config.client_id)?;
    }

    let project_root = project_root()?;
    let env_file = project_root.join("config/env_overrides/local-committee.env");
    if !env_file.exists() {
        bail!("config/env_overrides/local-committee.env not found at {}", env_file.display());
    }
    let env_map = load_env_file(&env_file)
        .with_context(|| format!("Failed to parse {}", env_file.display()))?;

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    // Keep all clients and join handles for monitors
    let mut clients: Vec<ManagedClient> = Vec::new();

    let ids: Vec<u8> = config.client_id.map_or_else(|| vec![1, 2, 3, 4], |id| vec![id]);
    validate_local_keystores(&ids)?;

    // launch clients and store in global state immediately
    let mut launch_error = None;
    for id in ids {
        let envs = build_env_for_client(id, &env_map, config.bitvmx_mode)?;
        let client_id = format!("client-{}", id);

        println!("============================================================================");
        println!("Launching client {} with env {:?}...", client_id, envs);
        println!("============================================================================");

        match launch_client_services(&config, envs, &client_id, &shutdown_tx) {
            Ok(services) => {
                let client = ManagedClient { client_id: client_id.to_string(), services };
                clients.push(client.clone());

                // store in global state for panic handler
                let clients_for_storage = clients.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(mut guard) = ACTIVE_CLIENTS.lock() {
                        *guard = Some(clients_for_storage);
                    }
                })
                .await;
            }
            Err(e) => {
                println!("Failed to launch all services for {client_id}");
                launch_error = Some(e);
                break;
            }
        }
    }

    // if launch failed, teardown and return error
    if let Some(err) = launch_error {
        eprintln!("Launch failed, tearing down already-started clients...");
        teardown_all(clients).await;
        return Err(err);
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

fn detect_and_kill_existing_services() -> Result<()> {
    println!("Checking for existing services...");

    // initialize system and refresh process list
    let mut sys =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut found_pids: Vec<(String, u32)> = Vec::new();

    // find all matching processes
    for service in &UNION_CLIENT_SERVICES {
        let service_name = service.name();

        for (pid, process) in sys.processes() {
            // check if the process name matches the service
            // the actual running process will be "target/debug/<service-name>" or just "<service-name>"
            if let Some(process_name) = process.name().to_str() {
                if process_name == service_name {
                    found_pids.push((service_name.to_string(), pid.as_u32()));
                }
            }
        }
    }

    if !found_pids.is_empty() {
        println!("Found {} running service instance(s)", found_pids.len());

        // group by service for better output
        let mut by_service: HashMap<String, Vec<u32>> = HashMap::new();
        for (service, pid) in &found_pids {
            by_service.entry(service.clone()).or_default().push(*pid);
        }

        for (service, pids) in &by_service {
            println!("  {}: PIDs {:?}", service, pids);
        }

        // shutdown coordinators first to allow them to unsubscribe from brokers gracefully
        let coordinator_name = Service::Coordinator.name();
        if let Some(coordinator_pids) = by_service.get(coordinator_name) {
            println!("Shutting down coordinators first...");
            for pid in coordinator_pids {
                let pid_val = Pid::from_raw(*pid as i32);
                if kill(pid_val, Signal::SIGTERM).is_ok() {
                    println!("  Sent SIGTERM to {} (PID {})", coordinator_name, pid);
                }
            }

            // wait for coordinators to exit (up to 10 seconds)
            let start = std::time::Instant::now();
            let coordinator_timeout = Duration::from_secs(10);
            loop {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                let still_alive: Vec<_> = coordinator_pids
                    .iter()
                    .filter(|&&pid| sys.process(sysinfo::Pid::from_u32(pid)).is_some())
                    .collect();

                if still_alive.is_empty() {
                    break;
                }

                if start.elapsed() >= coordinator_timeout {
                    // force kill any that didn't exit
                    for &&pid in &still_alive {
                        let pid_val = Pid::from_raw(pid as i32);
                        let _ = kill(pid_val, Signal::SIGKILL);
                        println!("  Force killed {} (PID {}, timeout)", coordinator_name, pid);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    break;
                }

                std::thread::sleep(Duration::from_millis(100));
            }

            // additional delay after coordinators exit to ensure cleanup completes
            println!("Coordinators stopped, waiting for cleanup...");
            std::thread::sleep(Duration::from_secs(2));
        }

        // now shutdown remaining services
        println!("Shutting down remaining services...");
        for (service, pid) in &found_pids {
            if service == coordinator_name {
                continue; // already handled
            }
            let pid_val = Pid::from_raw(*pid as i32);
            if kill(pid_val, Signal::SIGTERM).is_ok() {
                println!("  Sent SIGTERM to {} (PID {})", service, pid);
            }
        }

        // wait for graceful exit
        println!("Waiting for services to shut down...");
        std::thread::sleep(Duration::from_secs(2));

        // check which processes are still alive and force kill them
        sys.refresh_processes(ProcessesToUpdate::All, true);
        for (service, pid) in &found_pids {
            if service == coordinator_name {
                continue; // already handled
            }
            let pid_val = Pid::from_raw(*pid as i32);
            if sys.process(sysinfo::Pid::from_u32(*pid)).is_some() {
                let _ = kill(pid_val, Signal::SIGKILL);
                println!("  Force killed {} (PID {}, didn't exit gracefully)", service, pid);
            }
        }

        println!("All existing services cleaned up");
    } else {
        println!("No existing services found");
    }

    Ok(())
}

fn validate_1_4(value: u8, name: &str) -> Result<()> {
    if !(1..=4).contains(&value) {
        return Err(anyhow!("{} must be between 1 and 4", name));
    }
    Ok(())
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

fn join_base_storage_path(base_storage_path: &str, value: &str) -> String {
    if Path::new(value).is_absolute() {
        value.to_string()
    } else {
        format!("{base_storage_path}/{value}")
    }
}

fn project_root() -> Result<PathBuf> {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_path
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("Failed to determine project root"))
}

fn read_env_file_value(base_storage_path: &str, value: &str) -> Result<String> {
    let path = join_base_storage_path(base_storage_path, value);
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read broker metadata file {path}"))?;
    Ok(contents.trim().to_string())
}

fn union_bridge_path(base_storage_path: &str, value: &str) -> String {
    format!("{base_storage_path}/.union_bridge/{value}")
}

fn materialize_env_var(
    base_storage_path: &str,
    base_key: &str,
    value: &str,
) -> Result<Option<(String, String)>> {
    if base_key == "UB__COORDINATOR__BITVMX__PUBKEY_HASH_FILE" {
        let resolved = read_env_file_value(base_storage_path, value).with_context(|| {
            format!(
                "Missing BitVMX services pubkey hash file for local launch. \
Run `./cli-setup-operators.sh --ops 4` first (expected path from local-committee override: {value})"
            )
        })?;
        return Ok(Some(("UB__COORDINATOR__BITVMX__PUBKEY_HASH".to_string(), resolved)));
    }

    let (final_key, final_value) = if base_key == "UB__INDEXER__STORAGE__PATH" {
        (base_key.to_string(), union_bridge_path(base_storage_path, value))
    } else if base_key == "UB__COORDINATOR__STORAGE_PATH" {
        (base_key.to_string(), union_bridge_path(base_storage_path, value))
    } else if base_key == "UB__KEY_STORE__MEMBER_PATH" {
        (base_key.to_string(), union_bridge_path(base_storage_path, value))
    } else if base_key == "UB__KEY_STORE__USER_PATH" {
        (base_key.to_string(), union_bridge_path(base_storage_path, value))
    } else if matches!(
        base_key,
        "UB__BLOCK_INDEXER__BROKER_KEY_PATH"
            | "UB__LOG_INDEXER__BROKER_KEY_PATH"
            | "UB__USER_API__BROKER_KEY_PATH"
            | "UB__COORDINATOR__BROKER__KEY_PATH"
    ) {
        (base_key.to_string(), join_base_storage_path(base_storage_path, value))
    } else if let Some(target_key) = base_key.strip_suffix("_FILE") {
        (target_key.to_string(), read_env_file_value(base_storage_path, value)?)
    } else {
        (base_key.to_string(), value.to_string())
    };

    Ok(Some((final_key, final_value)))
}

fn build_env_for_client(
    id: u8,
    env_map: &HashMap<String, String>,
    bitvmx_mode: BitvmxMode,
) -> Result<Vec<(String, String)>> {
    validate_1_4(id, "CLIENT_ID")?;

    let base_storage_path = std::env::var("BASE_STORAGE_PATH")
        .context("BASE_STORAGE_PATH environment variable is required")?;

    let suffix = format!("_{}", id);
    let mut envs: Vec<(String, String)> = Vec::new();

    // iterate through all vars starting with UB__ and ending with the client suffix
    for (key, value) in env_map {
        if key.starts_with("UB__") && key.ends_with(&suffix) {
            // strip the _N suffix to get the base env var name
            let base_key = key.strip_suffix(&suffix).unwrap().to_string();
            if bitvmx_mode == BitvmxMode::Repo
                && base_key == "UB__COORDINATOR__BITVMX__PUBKEY_HASH_FILE"
            {
                // Repo mode runs BitVMX from repository configs, so per-operator docker artifacts
                // (.union_bridge/op_N/bitvmx/keys/services.pubkey_hash) may not exist.
                // We intentionally skip this *_FILE override here and let coordinator use
                // the default `coordinator.bitvmx.pubkey_hash` from config/base.toml.
                continue;
            }

            if let Some((final_key, final_value)) =
                materialize_env_var(&base_storage_path, &base_key, value)?
            {
                envs.push((final_key, final_value));
            }
        }
    }

    // add CLIENT_ID
    let client_id =
        env_map.get(&format!("CLIENT_ID_{}", id)).cloned().unwrap_or_else(|| id.to_string());
    envs.push(("CLIENT_ID".into(), client_id));

    if bitvmx_mode == BitvmxMode::Repo {
        envs.push((
            "UB__BRIDGE__COMMITTEE__DRP_PROGRAM_DEFINITION".to_string(),
            project_root()?.join("resources").join("union-verifier.yaml").display().to_string(),
        ));
    }

    Ok(envs)
}

fn validate_local_keystores(ids: &[u8]) -> Result<()> {
    let base_storage_path = std::env::var("BASE_STORAGE_PATH")
        .context("BASE_STORAGE_PATH environment variable is required")?;
    std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable is required for local client runs")?;

    for id in ids {
        let keystore_dir = Path::new(&base_storage_path)
            .join(".union_bridge")
            .join(format!("op_{id}"))
            .join("union-client")
            .join("keystore");

        for key_name in ["user", "member"] {
            let key_path = keystore_dir.join(key_name);
            if !key_path.exists() {
                bail!(
                    "Missing local keystore {}. Run `./cli-setup-operators.sh --ops 4` to recreate local artifacts.",
                    key_path.display()
                );
            }

            KeyManager::get_signer(&key_path).with_context(|| {
                format!(
                    "Failed to decrypt local {key_name} keystore {}. \
Check KEY_STORE_PASSWORD or rerun `./cli-setup-operators.sh --ops 4` if the keystore was created with a different password.",
                    key_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn fresh_cleanup(client_id: Option<u8>) -> Result<()> {
    let base_storage_path = std::env::var("BASE_STORAGE_PATH")
        .context("BASE_STORAGE_PATH environment variable is required")?;

    let union_bridge_root = Path::new(&base_storage_path).join(".union_bridge");

    if let Some(id) = client_id {
        let path_to_clean = union_bridge_root.join(format!("op_{id}")).join("local_database");
        if path_to_clean.exists() {
            fs::remove_dir_all(&path_to_clean)
                .with_context(|| format!("Failed to remove {}", path_to_clean.display()))?;
        }
    } else if union_bridge_root.exists() {
        for entry in fs::read_dir(&union_bridge_root)
            .with_context(|| format!("Failed to read {}", union_bridge_root.display()))?
        {
            let entry = entry?;
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if !file_name.starts_with("op_") {
                continue;
            }

            let database_path = entry.path().join("local_database");
            if database_path.exists() {
                fs::remove_dir_all(&database_path)
                    .with_context(|| format!("Failed to remove {}", database_path.display()))?;
            }
        }
    }

    Ok(())
}

fn cargo_args_for_service(config: &RunConfig, svc: &Service) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into(), "--bin".into(), svc.name().into()];

    // pass features flag if provided, otherwise default to anvil for services that support it
    let features = if let Some(f) = &config.features {
        Some(f.clone())
    } else {
        // default to anvil for local development on services that support it
        match svc {
            Service::BlockIndexer | Service::LogIndexer | Service::Coordinator => {
                Some("anvil".into())
            }
            Service::UserApi => None,
        }
    };

    if let Some(f) = features {
        args.push("--features".into());
        args.push(f);
    }

    args.push("--".into());
    args.push("--config".into());
    args.push("local".into());
    args
}

/// check if a port is listening
fn is_port_listening(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).map(|_| true).unwrap_or(false)
}

/// wait for a port to be listening, up to timeout
fn wait_for_port(port: u16, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if is_port_listening(port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("Port {} did not become available within {:?}", port, timeout)
}

/// extract port number from environment variables
fn get_port_from_envs(envs: &[(String, String)], key: &str) -> Option<u16> {
    envs.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.parse().ok())
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

        // coordinator depends on indexers being ready, wait for their broker ports
        if svc == Service::Coordinator {
            let block_port = get_port_from_envs(&envs, "UB__BLOCK_INDEXER__NOTIFIER__PORT");
            let log_port = get_port_from_envs(&envs, "UB__LOG_INDEXER__NOTIFIER__PORT");

            println!(
                "Waiting for indexers to be ready for {} (block port: {:?}, log port: {:?})...",
                client_id, block_port, log_port
            );

            // wait for both indexer broker ports to be listening (up to 60 seconds)
            if let Some(port) = block_port {
                wait_for_port(port, Duration::from_secs(60))
                    .with_context(|| format!("block-indexer broker not ready for {}", client_id))?;
                println!("  block-indexer broker ready on port {}", port);
            }
            if let Some(port) = log_port {
                wait_for_port(port, Duration::from_secs(60))
                    .with_context(|| format!("log-indexer broker not ready for {}", client_id))?;
                println!("  log-indexer broker ready on port {}", port);
            }

            println!("All indexers ready for {}, starting coordinator", client_id);
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
        let mgd_child = ManagedService { service: svc.name().to_string(), pid, child };
        services.push(mgd_child);
    }

    // Quick small delay to see if any exited immediately
    std::thread::sleep(Duration::from_secs(2));

    // Verify none exited immediately
    for ms in services.iter() {
        let name = format!("{}:{}", client_id, ms.service);
        let mut guard = ms.child.lock().expect(format!("Failed to lock {}", name).as_str());
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
                eprintln!("Process {} wait failed. Initiating shutdown...", monitor_name);
            }
            let _ = tx.send(());
        });
    }

    Ok(services)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn make_temp_dir() -> PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).expect("time").as_nanos();
        let path = std::env::temp_dir().join(format!("union-bridge-run-test-{unique}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn test_build_env_for_client_reads_pubkey_hash_file_references() {
        let _guard = TEST_MUTEX.lock().expect("lock");
        let base_storage_path = make_temp_dir();
        let hash_rel_path = ".union_bridge/op_1/union-client/broker/block-indexer.pubkey_hash";
        let hash_abs_path = base_storage_path.join(hash_rel_path);
        fs::create_dir_all(hash_abs_path.parent().expect("parent")).expect("mkdir");
        fs::write(&hash_abs_path, "abc123\n").expect("write hash file");

        std::env::set_var("BASE_STORAGE_PATH", &base_storage_path);

        let env_map = HashMap::from([
            (
                "UB__COORDINATOR__BLOCKS__PUBKEY_HASH_FILE_1".to_string(),
                ".union_bridge/op_1/union-client/broker/block-indexer.pubkey_hash".to_string(),
            ),
            (
                "UB__BLOCK_INDEXER__BROKER_KEY_PATH_1".to_string(),
                ".union_bridge/op_1/union-client/broker/block-indexer.pem".to_string(),
            ),
        ]);

        let envs = build_env_for_client(1, &env_map, BitvmxMode::Docker).expect("build envs");

        assert!(envs
            .contains(
                &("UB__COORDINATOR__BLOCKS__PUBKEY_HASH".to_string(), "abc123".to_string(),)
            ));
        assert!(envs.contains(&(
            "UB__BLOCK_INDEXER__BROKER_KEY_PATH".to_string(),
            base_storage_path
                .join(".union_bridge/op_1/union-client/broker/block-indexer.pem")
                .display()
                .to_string(),
        )));

        let _ = fs::remove_dir_all(base_storage_path);
        std::env::remove_var("BASE_STORAGE_PATH");
    }

    #[test]
    fn test_build_env_for_client_ignores_bitvmx_file_in_repo_mode() {
        let _guard = TEST_MUTEX.lock().expect("lock");
        let base_storage_path = make_temp_dir();

        std::env::set_var("BASE_STORAGE_PATH", &base_storage_path);

        let env_map = HashMap::from([
            (
                "UB__COORDINATOR__BITVMX__PUBKEY_HASH_FILE_1".to_string(),
                ".union_bridge/op_1/bitvmx/keys/services.pubkey_hash".to_string(),
            ),
            ("UB__COORDINATOR__BITVMX__PORT_1".to_string(), "22222".to_string()),
        ]);

        let envs = build_env_for_client(1, &env_map, BitvmxMode::Repo).expect("build envs");

        assert!(envs.iter().any(|(k, v)| k == "UB__COORDINATOR__BITVMX__PORT" && v == "22222"));
        assert!(!envs.iter().any(|(k, _)| k == "UB__COORDINATOR__BITVMX__PUBKEY_HASH"));

        let _ = fs::remove_dir_all(base_storage_path);
        std::env::remove_var("BASE_STORAGE_PATH");
    }
}

async fn teardown_all(clients: Vec<ManagedClient>) {
    // teardown all clients simultaneously, but within each client, stop services with proper ordering
    let mut handles = Vec::new();

    for client in clients {
        let handle = tokio::spawn(async move {
            println!("Tearing down {}", client.client_id);

            // shutdown coordinator first and wait for it to exit completely
            // this ensures it can properly unsubscribe from brokers before they shut down
            if let Some(coordinator) =
                client.services.iter().rev().find(|s| s.service == "coordinator")
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
