use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi, Identifier};
use common::msg_broker::types::{FromServer, MemberFundingInfo, ToServer};
use common::shutdown_flag::ShutdownFlag;
use common::types::Address;
use log::{error, info};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::time::{sleep, Instant};
use tower_http::timeout::TimeoutLayer;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{PeginAddressInput, RequestPegoutInput};
use uuid::Uuid;

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

#[derive(Serialize, Deserialize, Debug)]
pub struct AddressResponse {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MemberFundingInfoResponse {
    pub bitcoin_address: String,
    pub rsk_address: String,
}

struct FundingSyncBroker {
    broker: Arc<BrokerServer>,
    destination: Identifier,
    funding_info_state: Mutex<FundingInfoState>,
}

#[derive(Clone)]
enum FundingInfoState {
    Empty,
    Loading(Arc<Notify>),
    Ready(MemberFundingInfo),
}

impl FundingSyncBroker {
    async fn send(&self, msg: &FromServer) -> Result<(), (StatusCode, Json<Value>)> {
        self.broker.send(msg, &self.destination).map_err(internal_error)
    }

    async fn request_funding_info(&self) -> Result<MemberFundingInfo, (StatusCode, Json<Value>)> {
        loop {
            let waiter = {
                let mut state = self.funding_info_state.lock().await;
                match &*state {
                    FundingInfoState::Ready(info) => return Ok(info.clone()),
                    FundingInfoState::Loading(notify) => Some(notify.clone()),
                    FundingInfoState::Empty => {
                        let notify = Arc::new(Notify::new());
                        *state = FundingInfoState::Loading(notify.clone());
                        None
                    }
                }
            };

            if let Some(notify) = waiter {
                notify.notified().await;
                continue;
            }

            let result = self.fetch_funding_info().await;
            let mut state = self.funding_info_state.lock().await;
            let notify = match &*state {
                FundingInfoState::Loading(notify) => notify.clone(),
                FundingInfoState::Empty | FundingInfoState::Ready(_) => Arc::new(Notify::new()),
            };
            if let Ok(info) = &result {
                *state = FundingInfoState::Ready(info.clone());
            } else {
                *state = FundingInfoState::Empty;
            }
            notify.notify_waiters();
            return result;
        }
    }

    async fn fetch_funding_info(&self) -> Result<MemberFundingInfo, (StatusCode, Json<Value>)> {
        info!("Received member_funding_info for destination: {}", self.destination);
        let req_id = Uuid::new_v4();
        let deadline = Instant::now() + Duration::from_secs(9);

        self.broker
            .send(&FromServer::MemberRequest(req_id), &self.destination)
            .map_err(internal_error)?;

        loop {
            if Instant::now() >= deadline {
                return Err((
                    StatusCode::REQUEST_TIMEOUT,
                    Json(json!({ "error": "timed out waiting for member funding info" })),
                ));
            }

            let message = self.broker.try_recv().map_err(internal_error)?;

            match message {
                Some((ToServer::MemberFundingInfo(response_id, info), _sender))
                    if response_id == req_id =>
                {
                    return Ok(info);
                }
                Some((ToServer::BitVmxWalletError(response_id, error), _sender))
                    if response_id == req_id =>
                {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": error })),
                    ));
                }
                Some(_) | None => {
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
}

impl Server {
    pub async fn new<UCG>(
        listener: TcpListener,
        broker_server: Arc<BrokerServer>,
        shutdown_flag: ShutdownFlag,
        coordinator_client_id: Identifier,
        user_contracts_gateway: UCG,
    ) -> Self
    where
        UCG: RskContractsGatewayApi + Send + Sync + 'static,
    {
        // Wrap gateways for sync access
        let user_sync_gateway: Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi> =
            Arc::new(crate::sync_contracts_gateway::SyncContractsGateway::new(
                user_contracts_gateway,
            ));

        let funding_broker = Arc::new(FundingSyncBroker {
            broker: broker_server.clone(),
            destination: coordinator_client_id.clone(),
            funding_info_state: Mutex::new(FundingInfoState::Empty),
        });

        let mut app = Router::new().route("/health", get(Self::health_check));

        // User endpoints - public keys are now provided in request bodies
        app = app.nest(
            "/user",
            Router::new()
                .route("/pegin-address", post(Self::pegin_address))
                .route("/request-pegout", post(Self::request_pegout))
                .route("/rsk-address", get(Self::user_rsk_address))
                .layer(Extension(user_sync_gateway.clone())),
        );

        // Member endpoints - no bitcoin wallet needed, only broker communication
        app = app.nest(
            "/member",
            Router::new()
                .route("/apply-stream", post(Self::apply_stream))
                .layer(Extension(broker_server))
                .route("/funding-info", get(Self::member_funding_info))
                .layer(Extension(funding_broker)),
        );

        app = app.layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
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

    async fn member_funding_info(
        Extension(broker): Extension<Arc<FundingSyncBroker>>,
    ) -> impl IntoResponse {
        match broker.request_funding_info().await {
            Ok(info) => (
                StatusCode::OK,
                Json(json!(MemberFundingInfoResponse {
                    bitcoin_address: info.bitcoin_address,
                    rsk_address: info.rsk_address,
                })),
            ),
            Err(err) => err,
        }
    }

    async fn apply_stream(
        Extension(broker): Extension<Arc<FundingSyncBroker>>,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        info!(
            "Received apply stream request for destination: {} with payload: {:?}",
            broker.destination, payload
        );

        match broker.send(&FromServer::UserRequest(payload)).await {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(err) => err,
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

    async fn user_rsk_address(
        Extension(contracts): Extension<
            Arc<dyn crate::sync_contracts_gateway::SyncContractsGatewayApi>,
        >,
    ) -> impl IntoResponse {
        let address: Address = contracts.my_address();
        (StatusCode::OK, Json(json!(AddressResponse { address: address.to_string() })))
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
            Ok(tx_sent_output) => (
                StatusCode::OK,
                Json(json!({
                    "result": "ok",
                    "transaction_hash": tx_sent_output.transaction_hash,
                })),
            ),
            Err(e) => {
                error!("Error requesting pegout: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
            }
        }
    }
}

fn internal_error(err: impl ToString) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": err.to_string() })))
}

/// Validates a 32-byte X-only public key with 0x prefix
fn is_valid_xonly_pubkey(key: &str) -> bool {
    key.len() == 66 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Validates a 33-byte compressed public key with 0x prefix
fn is_valid_compressed_pubkey(key: &str) -> bool {
    key.len() == 68 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}
