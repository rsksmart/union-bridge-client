use crate::bitcoin::User;
use anyhow::{Context, Result};
use axum::routing::post;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Extension, Json, Router};
use bitcoin::secp256k1::rand::rngs::OsRng;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{secp256k1, PublicKey, XOnlyPublicKey};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi};
use common::msg_broker::types::FromServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

use serde::{Deserialize, Serialize};
use transaction_dispatcher::types::{PeginAddressInput, RequestPegoutInput};

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPeginInput {
    pub stream_amount: u64,
    pub packet_number: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UserRequestPegoutInput {
    pub amount_in_wei: u64,
}

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
}

impl Server {
    pub async fn new<CG: RskContractsGatewayApi + Send + Sync + 'static>(
        listener: TcpListener,
        broker_server: Arc<BrokerServer>,
        shutdown_flag: ShutdownFlag,
        coordinator_client_id: u32,
        contracts_gateway: CG,
        wallet_private_key: &str,
        network: bitcoin::Network,
    ) -> Self {
        let user = User::new(contracts_gateway.my_address(), wallet_private_key, network)
            .expect("Failed to create user");

        // Create sync wrapper that can work in any runtime context
        // NOTE: This uses a thread::spawn hack - should only be used in user-api
        let sync_gateway =
            crate::sync_contracts_gateway::SyncContractsGateway::new(contracts_gateway);
        let sync_gateway_arc: Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi> =
            Arc::new(sync_gateway);

        let app = Router::new()
            .route("/health", get(Self::health_check))
            .route("/bitvmx-address", get(Self::bitvmx_address))
            .route("/apply-stream", post(Self::apply_stream))
            .route("/pegin-address", post(Self::pegin_address))
            .route("/request-pegout", post(Self::request_pegout))
            .layer((
                TimeoutLayer::new(Duration::from_secs(10)),
                Extension(broker_server.clone()),
                Extension(coordinator_client_id),
                Extension(sync_gateway_arc),
                Extension(user),
            ));

        Self {
            listener,
            app,
            shutdown_flag,
        }
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
        Extension(destination): Extension<u32>,
    ) -> impl IntoResponse {
        info!("Received bitvmx_address for destination: {destination}",);

        // TODO(Jira) send a proper type in scope of https://rsklabs.atlassian.net/browse/UB-214
        let res = broker.send(&FromServer::MemberRequest, destination);
        match res {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ),
        }
    }

    async fn apply_stream(
        Extension(broker): Extension<Arc<BrokerServer>>,
        Extension(destination): Extension<u32>,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        info!(
            "Received apply stream request for destination: {} with payload: {:?}",
            destination, payload
        );

        // TODO(Jira) send a proper type instead of Value in scope of https://rsklabs.atlassian.net/browse/UB-214
        let res = broker.send(&FromServer::UserRequest(payload), destination);
        match res {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ),
        }
    }

    async fn pegin_address(
        Extension(user): Extension<User>,
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(mut payload): Json<PeginAddressInput>,
    ) -> impl IntoResponse {
        info!("Received pegin-address request: {payload:?}");

        // Use our own X-only public key if not provided
        if payload.btc_reimbursement_pub_key.is_empty() {
            let x_only_key = XOnlyPublicKey::from(user.bitcoin_public_key);
            payload.btc_reimbursement_pub_key = format!("0x{}", x_only_key);
        }

        match contracts.get_temporary_pegin_address(payload) {
            Ok(data) => (StatusCode::OK, Json(json!(data))),
            Err(e) => {
                error!("Error requesting pegin: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
            }
        }
    }

    async fn request_pegout(
        Extension(user): Extension<User>,
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(payload): Json<UserRequestPegoutInput>,
    ) -> impl IntoResponse {
        info!("Received request_pegout request: {:?}", payload);
        let usr_pub_key = format!("0x{}", user.bitcoin_public_key);

        let amount_in_wei = payload.amount_in_wei;
        info!(
            "Request pegout -> usr_pub_key: {} amount_in_wei: {}",
            usr_pub_key, amount_in_wei
        );
        let input = RequestPegoutInput {
            amount_in_wei,
            usr_pub_key,
        };
        let res = contracts.request_pegout(input);
        match res {
            Ok(_tx_sent_output) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => {
                error!("Error requesting pegout: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
            }
        }
    }

    pub fn get_random_pubkey() -> PublicKey {
        let secp = secp256k1::Secp256k1::new();
        let mut rng = OsRng;
        let too_sk = SecretKey::new(&mut rng);
        let too_pk = secp256k1::PublicKey::from_secret_key(&secp, &too_sk);
        PublicKey {
            compressed: true,
            inner: too_pk,
        }
    }
}
