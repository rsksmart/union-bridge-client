use crate::bitcoin::{BitcoinClient, User};
use anyhow::{Context, Result};
use axum::routing::post;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Extension, Json, Router};
use bitcoin::secp256k1::rand::rngs::OsRng;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{secp256k1, PublicKey, XOnlyPublicKey};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi};
use common::msg_broker::types::FromServer;
use common::shutdown_flag::ShutdownFlag;
use common::types::ToHexString;
use log::{error, info};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

use serde::{Deserialize, Serialize};
use transaction_dispatcher::types::PeginAddressInput;

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPeginInput {
    pub stream_amount: u64,
    pub packet_number: Option<u64>,
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
        bitcoin_client: BitcoinClient,
    ) -> Self {
        let user = User::new(contracts_gateway.my_address(), bitcoin_client)
            .expect("Failed to create user");

        // Create sync wrapper that can work in any runtime context
        // NOTE: This uses a thread::spawn hack - should only be used in user-api
        let sync_gateway =
            crate::sync_contracts_gateway::SyncContractsGateway::new(contracts_gateway);
        let sync_gateway_arc: Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi> =
            Arc::new(sync_gateway);

        let app = Router::new()
            .route("/health", get(Self::health_check))
            .route("/apply-stream", post(Self::apply_stream))
            .route("/request-pegin", post(Self::request_pegin))
            .route("/pegin-address", post(Self::pegin_address))
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

    async fn request_pegin(
        Extension(user): Extension<User>,
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(payload): Json<RequestPeginInput>,
    ) -> impl IntoResponse {
        info!("Received request_pegin request: {:?}", payload);

        let stream_value = payload.stream_amount;
        // TODO how does the user know which packet number to use? it's not part of getTemporaryAddress response
        let packet_number = payload.packet_number.unwrap_or(0);

        let x_only_key = XOnlyPublicKey::from(user.public_key);

        let tmp_addr_call = contracts.get_temporary_pegin_address(PeginAddressInput {
            rootstock_deposit_address: user.rsk_address.to_hex_string(),
            value: stream_value,
            btc_reimbursement_pub_key: format!("0x{}", x_only_key),
        });

        let tmp_addr = match tmp_addr_call {
            Ok(res) => {
                info!("Got temporary pegin address");
                res.address
            }
            Err(e) => {
                error!("Error getting temporary pegin address: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                );
            }
        };

        // Run Bitcoin RPC calls in a blocking context to avoid runtime drop panic
        let res = tokio::task::spawn_blocking(move || {
            user.request_pegin(stream_value, packet_number, tmp_addr)
        })
        .await;

        match res {
            Ok(Ok(_)) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Ok(Err(e)) => {
                error!("Error requesting pegin: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
            }
            Err(e) => {
                error!("Error requesting pegin: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("Task join error: {}", e) })),
                )
            }
        }
    }

    async fn pegin_address(
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
        Json(payload): Json<PeginAddressInput>,
    ) -> impl IntoResponse {
        info!(
            "Received pegin-address request: amount={}, packet_number={:?}",
            payload.amount, payload.packet_number
        );

        match contracts.get_temporary_pegin_address(payload) {
            Ok(data) => (StatusCode::OK, Json(json!(data))),
            Err(e) => {
                error!("Error getting temporary pegin address: {e}");
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
