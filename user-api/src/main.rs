use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::BrokerServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use tokio::net::TcpListener;
use transaction_dispatcher::config::Config as TxDispatcherConfig;
use user_api::config::{Config, Logger};
use user_api::Server;

const LOGGER_CLI_FLAG: &str = "logger-path";
const ENV_CLI_FLAG: &str = "env";

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
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(ENV_CLI_FLAG)
                .short('e')
                .long(ENV_CLI_FLAG)
                .value_name("ENV")
                .help("Environment name (e.g., local, alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>(ENV_CLI_FLAG).cloned();

    info!(
        "Loading configuration for environment: {}",
        env_name.clone().unwrap_or_else(|| "NONE".to_string())
    );
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(env_name.clone()).expect("Failed to load config");

    info!("Starting user-api server");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = config.user_api_config.notifier.port;
    let broker_storage_path = &config.user_api_config.notifier.broker_storage_path;
    let broker_drop_guard = BrokerDropGuard::new(Arc::new(
        BrokerServer::new(broker_port, broker_storage_path)
            .context("Failed to create user-api broker server")?,
    ));
    info!("Broker Server started on {broker_port}");

    let http_addr = SocketAddr::from(([0, 0, 0, 0], config.user_api_config.http.port));
    let listener = TcpListener::bind(http_addr).await.context("Failed to bind to address")?;

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: TxDispatcherConfig =
        TxDispatcherConfig::load(env_name).expect("Failed to load transaction dispatcher config");

    // Create two contract gateways with different roles
    let user_contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib(
        tx_dispatcher_config.clone(),
        transaction_dispatcher::GatewayRole::User,
    )
    .await?;

    let member_contracts_gateway = transaction_dispatcher::get_contracts_gateway_as_lib(
        tx_dispatcher_config,
        transaction_dispatcher::GatewayRole::Member,
    )
    .await?;

    let server = Server::new(
        listener,
        broker_drop_guard.clone_arc().context("failed to clone broker server handle")?,
        shutdown_flag.clone(),
        config.user_api_config.coordinator.broker.client_id,
        user_contracts_gateway,
        member_contracts_gateway,
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
