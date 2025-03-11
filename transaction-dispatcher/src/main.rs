use alloy_provider::{ProviderBuilder, RootProvider, WsConnect};
use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::config::Config;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use std::sync::Arc;
use transaction_dispatcher::rsk_connector::RskContractsGatewayAlloy;
use transaction_dispatcher::server::Server;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

#[tokio::main]
async fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Transaction Dispatcher")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file")
                .default_value("../log4rs.yaml"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory")
                .default_value("../config/dev"),
        )
        .get_matches();

    let logger_path: &String = matches.get_one(LOGGER_CLI_FLAG).unwrap();
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let config_path: &String = matches.get_one(CONFIG_CLI_FLAG).unwrap();
    let config = Config::load(config_path).expect("Failed to load config");

    let shutdown_flag = ShutdownFlag::init();

    let ws = WsConnect::new(&config.provider.rootstock.url);
    let provider: RootProvider = ProviderBuilder::default().on_ws(ws).await?;

    info!(
        "Connected to Rootstock at {}",
        &config.provider.rootstock.url
    );

    let rsk_contract_gateway = Arc::new(
        RskContractsGatewayAlloy::new(&provider, &config)
            .context("Could not instantiate RskContractsGateway")?,
    );

    let listener =
        tokio::net::TcpListener::bind(&config.transaction_dispatcher.server_address).await?;

    let server = Server::new(listener, rsk_contract_gateway, shutdown_flag).await;

    let server_handle = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            error!("Server error: {}", e);
        }
    });

    server_handle.await?;

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}
