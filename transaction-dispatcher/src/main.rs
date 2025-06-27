use alloy_provider::network::EthereumWallet;
use alloy_provider::{ProviderBuilder, WsConnect};
use anyhow::{Context, Result};
use clap::{Arg, Command};
use common::shutdown_flag::ShutdownFlag;
use key_manager::key_manager::KeyManager;
use log::{error, info};
use std::path::Path;
use transaction_dispatcher::config::{ConfigAsBin, Logger};
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
    let config: ConfigAsBin = ConfigAsBin::load(config_path).expect("Failed to load config");

    let shutdown_flag = ShutdownFlag::init();

    let ws = WsConnect::new(&config.provider().rootstock.url);

    let key_store_path = Path::new(&config.key_store().path);

    info!("Getting signer from key at {}", key_store_path.display());
    let signer = KeyManager::get_signer(key_store_path)?;
    let address = signer.address().to_string();
    info!("Got signer with address {address}");

    let wallet = EthereumWallet::from(signer);

    let provider = ProviderBuilder::new()
        .wallet(wallet.clone())
        .with_simple_nonce_management()
        .connect_ws(ws)
        .await
        .context("Failed to connect to Rootstock provider")?;

    info!(
        "Connected to Rootstock at {} with address {}",
        &config.provider().rootstock.url,
        address
    );

    let rsk_contract_gateway = RskContractsGateway::new(
        provider,
        config.load_managed_contracts(),
        config.transaction(),
    )
    .context("Could not instantiate RskContractsGateway")?;

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
