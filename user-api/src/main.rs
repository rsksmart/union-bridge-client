use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::config::CommonConfig;
use common::msg_broker::broker::BrokerServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use transaction_dispatcher::config::ConfigAsLib as TxDispatcherConfig;
use user_api::config::{Config, Logger};
use user_api::Server;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

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
        .arg(
            Arg::new("env")
                .short('e')
                .long("env")
                .value_name("ENV")
                .help("Environment name (e.g., anvil, alphanet, stage)"),
        )
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let env_name = matches.get_one::<String>("env").cloned();

    info!("Loading configuration from environment files");
    info!("Environment variables with prefix UB__ will override config values");

    let config: Config = Config::load(env_name.clone()).expect("Failed to load config");

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: TxDispatcherConfig =
        TxDispatcherConfig::load(env_name).expect("Failed to load transaction dispatcher config");

    let contracts_gateway =
        transaction_dispatcher::get_contracts_gateway_as_lib(tx_dispatcher_config).await?;

    info!("Starting user-api server");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = config.broker_server_port;
    let broker_server = Arc::new(BrokerServer::new(broker_port));
    info!("Broker Server started on {broker_port}");

    // Create the broker shutdown task early to ensure proper cleanup even on early errors
    let broker_fut = {
        let broker_server = broker_server.clone();
        let shutdown_flag = shutdown_flag.clone();
        tokio::spawn(async move {
            shutdown_flag.wait_for().await;
            tokio::task::spawn_blocking(move || {
                // Note: BrokerServer doesn't need explicit close() call
                // The runtime will be dropped when the Arc is dropped
                drop(broker_server); // <- drop in blocking context
                info!("Shutdown requested, Broker server closed");
            })
            .await
            .expect("failed to drop broker server");
        })
    };

    let http_addr = SocketAddr::from(([0, 0, 0, 0], config.http_server_port));
    let listener = TcpListener::bind(http_addr)
        .await
        .context("Failed to bind to address")?;

    let network = CommonConfig::parse_bitcoin_network(&config.bitcoin_network)
        .context("Failed to parse bitcoin_network")?;

    // Get wallet private key from environment variable (same as bitcoin-wallet)
    let wallet_private_key = std::env::var("WALLET_PRIVATE_KEY")
        .context("WALLET_PRIVATE_KEY environment variable not set")?;

    let server = Server::new(
        listener,
        broker_server.clone(),
        shutdown_flag.clone(),
        config.coordinator_broker_client_id,
        contracts_gateway,
        &wallet_private_key,
        network,
    )
    .await;

    info!("Http Server started, listening on {http_addr}");
    if let Err(err) = server.start().await {
        error!("Server error: {}", err);
        return Err(err);
    }

    // Wait for the broker shutdown task to complete
    broker_fut
        .await
        .inspect_err(|e| error!("Error closing Broker Server: {}", e))?;

    info!("Shutdown complete");

    Ok(())
}
