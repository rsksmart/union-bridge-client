use alloy_provider::{ProviderBuilder, RootProvider, WsConnect};
use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};
use clap::{Arg, Command};
use common::config::Config;
use log::info;
use std::sync::Arc;
use transaction_dispatcher::rsk_connector::RskContractsGateway;
use transaction_dispatcher::types::{PeginAddressInput, PeginAddressOutput};

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

#[tokio::main]
async fn main() -> Result<()> {
    let matches = Command::new("Union Bridge Block Indexer")
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

    let ws = WsConnect::new(&config.provider.rootstock.url);
    let provider: RootProvider = ProviderBuilder::default().on_ws(ws).await?;

    info!(
        "Connected to Rootstock at {}",
        &config.provider.rootstock.url
    );

    let rsk_contract_gateway = Arc::new(
        RskContractsGateway::new(&provider, &config)
            .context("Could not instantiate RskContractsGateway")?,
    );

    let app = Router::new()
        .route("/pegin-address", post(create_pegin_address))
        .layer(Extension(rsk_contract_gateway.clone()));

    let listener =
        tokio::net::TcpListener::bind(&config.transaction_dispatcher.server_address).await?;

    axum::serve(listener, app).await?;

    // TODO(iago) move server logic to new file

    // TODO(iago) graceful shutdown: https://github.com/tokio-rs/axum/blob/da3539cb0e5eed381361b2e688a776da77c52cd6/examples/graceful-shutdown/src/main.rs#L38

    info!("Quitting now...");
    log::logger().flush();

    Ok(())
}

async fn create_pegin_address(
    Extension(rsk_gateway): Extension<Arc<RskContractsGateway>>,
    Json(payload): Json<PeginAddressInput>,
) -> (StatusCode, Json<PeginAddressOutput>) {
    match rsk_gateway.get_temporary_pegin_address(payload).await {
        Ok(address) => (StatusCode::CREATED, Json(address)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PeginAddressOutput {
                address: e.to_string(),
            }),
        ),
    }
}
