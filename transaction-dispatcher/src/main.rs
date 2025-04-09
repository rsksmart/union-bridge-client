use alloy_provider::network::EthereumWallet;
use alloy_provider::{ProviderBuilder, WsConnect};
use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::shutdown_flag::ShutdownFlag;
use key_manager::key_manager::KeyManager;
use log::{error, info};
use std::path::Path;
use std::sync::Arc;
use transaction_dispatcher::config::{Config, Logger};
use transaction_dispatcher::rsk_gateway::RskContractsGateway;
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

    let shutdown_flag = ShutdownFlag::init();

    let ws = WsConnect::new(&config.provider.rootstock.url);

    let key_store_path = Path::new(&config.key_store.path);
    let key_store_password = std::env::var("KEY_STORE_PASSWORD")
        .context("KEY_STORE_PASSWORD environment variable not found")?;
    let signer = KeyManager::get_signer(key_store_path, key_store_password)?;
    let address = signer.address().to_string();
    let wallet = EthereumWallet::from(signer);

    let provider = ProviderBuilder::new()
        .wallet(wallet.clone())
        .on_ws(ws)
        .await?;

    info!(
        "Connected to Rootstock at {} with address {}",
        &config.provider.rootstock.url, address
    );

    let rsk_contract_gateway = Arc::new(
        RskContractsGateway::new(
            provider,
            config.load_managed_contracts(),
            &config.transaction,
        )
        .context("Could not instantiate RskContractsGateway")?,
    );

    let listener = tokio::net::TcpListener::bind(&config.server.url).await?;

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
