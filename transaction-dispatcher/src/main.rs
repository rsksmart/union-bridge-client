use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Arg, Command};
use common::config::Config;
use transaction_dispatcher::rsk_connector;
use transaction_dispatcher::types::{PeginAddressInput, PeginAddressOutput};

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";

#[tokio::main]
async fn main() {
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

    let app = Router::new()
        .route("/", get(root))
        .route("/pegin-address", post(create_pegin_address));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// TODO temporary helper until we have Swagger/OpenAPI
async fn root() -> &'static str {
    "Use POST /pegin-address (rootstock_deposit_address, value, btc_reimbursement_pub_key) to get the temporary peg-in address\n"
}

async fn create_pegin_address(
    Json(payload): Json<PeginAddressInput>,
) -> (StatusCode, Json<PeginAddressOutput>) {
    // TODO(iago) validate input

    // TODO(iago) map errors to status codes
    match rsk_connector::get_temporary_pegin_address(payload).await {
        Ok(address) => (StatusCode::CREATED, Json(address)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PeginAddressOutput {
                address: e.to_string(),
            }),
        ),
    }
}
