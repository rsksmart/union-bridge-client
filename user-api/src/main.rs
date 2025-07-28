use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::BrokerServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use user_api::config::{Config, Logger};
use user_api::Server;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

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
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_path = matches.get_one::<String>(CONFIG_CLI_FLAG);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    info!("Starting user-api server");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = config.broker_server_port;
    let broker_server = Arc::new(BrokerServer::new(broker_port));
    info!("Broker Server started on {broker_port}");

    let http_addr = SocketAddr::from(([0, 0, 0, 0], config.http_server_port));
    let listener = TcpListener::bind(http_addr)
        .await
        .context("Failed to bind to address")?;
    let server = Server::new(
        listener,
        broker_server.clone(),
        shutdown_flag.clone(),
        config.coordinator_broker_client_id,
    )
    .await;
    info!("Http Server started, listening on {http_addr}");
    if let Err(err) = server.start().await {
        error!("Server error: {}", err);
        return Err(err);
    }

    // this is required due to BrokerServer creating its own runtime, and it cannot be dropped in an async context: Cannot drop a runtime in a context where blocking is not allowed
    tokio::spawn(async move {
        shutdown_flag.wait_for().await;

        tokio::task::spawn_blocking(move || {
            drop(broker_server); // <- drop in blocking context
        })
        .await
        .expect("failed to drop broker server");
    });

    info!("Server shutdown complete");

    Ok(())
}
