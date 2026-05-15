use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use anyhow::{ensure, Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::{BrokerServer, Identifier};
use common::shutdown_flag::ShutdownFlag;
use tokio::net::TcpListener;
use tracing::{error, info};
use transaction_dispatcher::config::Config as TxDispatcherConfig;
use user_api::config::{Config, Logger};
use user_api::Server;

const LOG_DIR_CLI_FLAG: &str = "log-dir";
const CONFIG_CLI_FLAG: &str = "config";

struct BrokerDropGuard(Option<Arc<BrokerServer>>);

impl BrokerDropGuard {
    fn new(broker: Arc<BrokerServer>) -> Self {
        Self(Some(broker))
    }

    fn clone_arc(&self) -> Option<Arc<BrokerServer>> {
        self.0.as_ref().map(Arc::clone)
    }
}

impl Drop for BrokerDropGuard {
    fn drop(&mut self) {
        if let Some(broker) = self.0.take() {
            let spawn_result =
                thread::Builder::new().name("broker-drop".into()).spawn(move || drop(broker));
            if let Err(err) = spawn_result {
                error!("Failed to spawn broker drop thread: {}", err);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let matches = Command::new("Union Bridge User API")
        .arg(
            Arg::new(LOG_DIR_CLI_FLAG)
                .short('l')
                .long(LOG_DIR_CLI_FLAG)
                .value_name("DIR")
                .help("Directory for log files (also set via UB_LOG_DIR). Omit for stdout only."),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('e')
                .long(CONFIG_CLI_FLAG)
                .value_name("CONFIG")
                .help("Configuration profile name (e.g., local, docker, alphanet)"),
        )
        .get_matches();

    let log_dir = matches.get_one::<String>(LOG_DIR_CLI_FLAG);
    let _log_guard = Logger::init(log_dir).expect("Failed to load logger");

    let config_file = matches.get_one::<String>(CONFIG_CLI_FLAG).cloned();

    info!(
        "Loading configuration profile: {}",
        config_file.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(config_file.clone()).expect("Failed to load config");

    info!("Starting user-api server");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = config.user_api_config.notifier.port;
    let broker_key_path = config.user_api_config.broker_key_path.clone();

    let broker_server =
        tokio::task::spawn_blocking(move || BrokerServer::new(broker_port, &broker_key_path))
            .await
            .context("Failed to spawn blocking task")?
            .context("Failed to create BrokerServer")?;

    let broker_drop_guard = BrokerDropGuard::new(Arc::new(broker_server));
    info!("Broker Server started on {broker_port}");

    let http_addr = SocketAddr::from(([0, 0, 0, 0], config.user_api_config.http.port));
    let listener = TcpListener::bind(http_addr).await.context("Failed to bind to address")?;

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: TxDispatcherConfig = TxDispatcherConfig::load(config_file)
        .expect("Failed to load transaction dispatcher config");

    let user_contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib(
        tx_dispatcher_config.clone(),
        transaction_dispatcher::GatewayRole::User,
    )
    .await?;

    let pubkey_hash = config.user_api_config.coordinator.pubkey_hash.clone();
    ensure!(
        !pubkey_hash.is_empty() && pubkey_hash != "<to_patch_with_env>",
        "coordinator must be configured with the coordinator broker pubkey_hash"
    );

    let coordinator_client_id = Identifier::new(
        pubkey_hash,
        u8::try_from(config.user_api_config.coordinator.client_id)
            .context("user_api.coordinator.client_id must fit in u8")?,
    );

    let admin_token = config.user_api_config.admin_token.clone();

    let server = Server::new(
        listener,
        broker_drop_guard.clone_arc().context("failed to clone broker server handle")?,
        shutdown_flag.clone(),
        coordinator_client_id,
        user_contracts_gateway,
        admin_token,
    )
    .await;

    info!("Http Server started, listening on {http_addr}");
    let server_result = server.start().await;

    drop(broker_drop_guard);

    if let Err(err) = server_result {
        error!("Server error: {}", err);
        return Err(err);
    }

    info!("Shutdown complete");

    Ok(())
}
