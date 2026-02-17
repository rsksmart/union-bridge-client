use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use bitcoin::secp256k1::rand::rngs::OsRng;
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::PublicKey;
use common::msg_broker::broker::{BrokerServer, BrokerServerApi, Identifier};
use common::msg_broker::types::FromServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{PeginAddressInput, RequestPegoutInput};

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPeginInput {
    pub stream_amount: u64,
    pub packet_number: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UserRequestPegoutInput {
    pub amount_in_wei: u64,
    pub usr_pub_key: String,
}

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
}

impl Server {
    pub async fn new<UCG, MCG>(
        listener: TcpListener,
        broker_server: Arc<BrokerServer>,
        shutdown_flag: ShutdownFlag,
        coordinator_client_id: Identifier,
        user_contracts_gateway: UCG,
        member_contracts_gateway: MCG,
    ) -> Self
    where
        UCG: RskContractsGatewayApi + Send + Sync + 'static,
        MCG: RskContractsGatewayApi + Send + Sync + 'static,
    {
        // Wrap gateways for sync access
        let user_sync_gateway: Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi> =
            Arc::new(crate::sync_contracts_gateway::SyncContractsGateway::new(
                user_contracts_gateway,
            ));
        let member_sync_gateway: Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi> =
            Arc::new(crate::sync_contracts_gateway::SyncContractsGateway::new(
                member_contracts_gateway,
            ));

        let mut app = Router::new().route("/health", get(Self::health_check));

        // User endpoints - public keys are now provided in request bodies
        app = app.nest(
            "/user",
            Router::new()
                .route("/pegin-address", post(Self::pegin_address))
                .route("/request-pegout", post(Self::request_pegout))
                .layer(Extension(user_sync_gateway.clone())),
        );

        // Member endpoints - no bitcoin wallet needed, only broker communication
        app = app.nest(
            "/member",
            Router::new()
                .route("/apply-stream", post(Self::apply_stream))
                .route("/bitvmx-address", get(Self::bitvmx_address))
                .layer(Extension(member_sync_gateway.clone())),
        );

        app = app.layer((
            TimeoutLayer::new(Duration::from_secs(10)),
            Extension(broker_server.clone()),
            Extension(coordinator_client_id),
        ));

        Self { listener, app, shutdown_flag }
    }

    pub async fn start(self) -> Result<()> {
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(self.shutdown_flag.wait_for())
            .await
            .context("Error starting server")
    }

    async fn health_check() -> impl IntoResponse {
        (StatusCode::OK, Json(json!({ "status": "ok" })))
    }

    async fn bitvmx_address(
        Extension(broker): Extension<Arc<BrokerServer>>,
        Extension(destination): Extension<Identifier>,
    ) -> impl IntoResponse {
        info!("Received bitvmx_address for destination: {destination}",);

        // TODO(Jira) send a proper type in scope of https://rsklabs.atlassian.net/browse/UB-214
        let res = broker.send(&FromServer::MemberRequest, &destination);
        match res {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        }
    }

    async fn apply_stream(
        Extension(broker): Extension<Arc<BrokerServer>>,
        Extension(destination): Extension<Identifier>,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        info!(
            "Received apply stream request for destination: {} with payload: {:?}",
            destination, payload
        );

        // TODO(Jira) send a proper type instead of Value in scope of https://rsklabs.atlassian.net/browse/UB-214
        let res = broker.send(&FromServer::UserRequest(payload), &destination);
        match res {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        }
    }

    async fn pegin_address(
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(payload): Json<PeginAddressInput>,
    ) -> impl IntoResponse {
        info!("Received pegin-address request: {payload:?}");

        // Validate btc_reimbursement_pub_key is provided
        if payload.btc_reimbursement_pub_key.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "btc_reimbursement_pub_key is required" })),
            );
        }

        // Validate format: must be 0x + 64 hex chars (32 bytes x-only pubkey)
        if !is_valid_xonly_pubkey(&payload.btc_reimbursement_pub_key) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": "btc_reimbursement_pub_key must be a valid 32-byte hex string with 0x prefix (66 chars total)" }),
                ),
            );
        }

        match contracts.get_temporary_pegin_address(payload) {
            Ok(data) => (StatusCode::OK, Json(json!(data))),
            Err(e) => {
                error!("Error requesting pegin: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
            }
        }
    }

    async fn request_pegout(
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(payload): Json<UserRequestPegoutInput>,
    ) -> impl IntoResponse {
        info!("Received request_pegout request: {:?}", payload);

        // Validate usr_pub_key is provided
        if payload.usr_pub_key.is_empty() {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "usr_pub_key is required" })));
        }

        // Validate format: must be 0x + 66 hex chars (33 bytes compressed pubkey)
        if !is_valid_compressed_pubkey(&payload.usr_pub_key) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "error": "usr_pub_key must be a valid 33-byte compressed public key hex string with 0x prefix (68 chars total)" }),
                ),
            );
        }

        let amount_in_wei = payload.amount_in_wei;
        let usr_pub_key = payload.usr_pub_key;
        info!("Request pegout -> usr_pub_key: {} amount_in_wei: {}", usr_pub_key, amount_in_wei);
        let input = RequestPegoutInput { amount_in_wei, usr_pub_key };
        let res = contracts.request_pegout(input);
        match res {
            Ok(_tx_sent_output) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => {
                error!("Error requesting pegout: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
            }
        }
    }

    pub fn get_random_pubkey() -> PublicKey {
        let secp = Secp256k1::new();
        let mut rng = OsRng;
        let too_sk = SecretKey::new(&mut rng);
        let too_pk = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &too_sk);
        PublicKey { compressed: true, inner: too_pk }
    }
}

/// Validates a 32-byte X-only public key with 0x prefix
fn is_valid_xonly_pubkey(key: &str) -> bool {
    key.len() == 66 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Validates a 33-byte compressed public key with 0x prefix
fn is_valid_compressed_pubkey(key: &str) -> bool {
    key.len() == 68 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}
