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

use crate::bitcoin::KeyType::XOnlyKey;
use transaction_dispatcher::types::PeginAddressInput;

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
        // TODO(iago) should match the address used by getTemporaryAddress
        let user = User::new(
            "e49f8edd018329f77037b58fcd98bc719798da49"
                .try_into()
                .expect("Invalid address"),
            bitcoin_client,
        )
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

    // NOTE: this is a temporary implementation, just to have something working
    async fn request_pegin(
        Extension(user): Extension<User>,
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
    ) -> impl IntoResponse {
        info!("Received request_pegin request for destination");

        let rsk_address = contracts.my_address();

        // TODO(iago) get from user input
        let stream_value = 1_000_000;
        // TODO how does the user know which packet number to use? it's not part of getTemporaryAddress response
        let packet_number = 0;

        let xonly = XOnlyPublicKey::from(user.public_key);

        let tmp_addr_call = contracts.get_temporary_pegin_address(PeginAddressInput {
            rootstock_deposit_address: rsk_address.to_hex_string(),
            value: stream_value,
            btc_reimbursement_pub_key: format!("0x{}", xonly),
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
