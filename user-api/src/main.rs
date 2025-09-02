use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi};
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use transaction_dispatcher::config::ConfigAsLib;
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
        .get_matches();

    let logger_cfg_path = matches.get_one::<String>(LOGGER_CLI_FLAG);
    Logger::init(logger_cfg_path).expect("Failed to load logger");

    let config_path = matches.get_one::<String>(CONFIG_CLI_FLAG);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    // Load transaction dispatcher configuration
    let tx_dispatcher_config: ConfigAsLib =
        ConfigAsLib::load(config_path).expect("Failed to load transaction dispatcher config");

    let contracts_gateway =
        transaction_dispatcher::get_contracts_gateway_as_lib(tx_dispatcher_config)
            .await
            .expect("Failed to get contracts gateway");

    info!("Starting user-api server");

    // Create BitcoinClient in blocking context to avoid runtime drop panic (we are in a Tokio context)
    let bitcoin_client =
        tokio::task::spawn_blocking(|| user_api::bitcoin::build_bitcoin_client_regtest())
            .await
            .expect("Failed to spawn blocking task for Bitcoin client creation");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = config.broker_server_port;
    let mut broker_server = Arc::new(BrokerServer::new(broker_port));
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
        contracts_gateway,
        bitcoin_client,
    )
    .await;

    info!("Http Server started, listening on {http_addr}");
    if let Err(err) = server.start().await {
        error!("Server error: {}", err);
        return Err(err);
    }

    // this is required due to BrokerServer creating its own runtime, and it cannot be dropped in an async context: Cannot drop a runtime in a context where blocking is not allowed
    let broker_fut = tokio::spawn(async move {
        shutdown_flag.wait_for().await;

        tokio::task::spawn_blocking(move || {
            Arc::get_mut(&mut broker_server)
                .expect("Could not get Broker Server to close")
                .close();
            drop(broker_server); // <- drop in blocking context
            info!("Shutdown requested, Broker server closed");
        })
        .await
        .expect("failed to drop broker server");
    });

    broker_fut
        .await
        .inspect_err(|e| error!("Error closing Broker Server: {}", e))?;

    info!("Shutdown complete");

    Ok(())
}
