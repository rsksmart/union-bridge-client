use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi, Identifier};
use common::msg_broker::types::{FromServer, MemberFundingInfo, ToServer};
use common::shutdown_flag::ShutdownFlag;
use common::types::Address;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::time::{sleep, Instant};
use tower_http::timeout::TimeoutLayer;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{PeginAddressInput, RequestPegoutInput};
use uuid::Uuid;

use crate::errors::ApiError;

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

            // This endpoint is currently the only user-api consumer of broker replies, and the
            // expected inbound messages are funding-info responses keyed by req_id. If more
            // request/response flows are added to this broker, move try_recv into a shared
            // router that demultiplexes replies by UUID instead of polling from each endpoint.
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

#[derive(Deserialize, Debug)]
pub struct FailFlowReq {
    pub kind: String,
    pub flow_id: Uuid,
    pub reason: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ApplyStreamReq {
    #[serde(rename = "ApplyToStream")]
    pub apply_to_stream: Value,
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
        admin_token: Option<String>,
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
                .route("/funding-info", get(Self::member_funding_info))
                .layer(Extension(funding_broker.clone())),
        );

        // Admin endpoints. Auth is route middleware so unauthenticated requests are
        // rejected before handler extractors read or parse the body.
        let admin_token_for_middleware = admin_token.clone();
        app = app.nest(
            "/admin",
            Router::new()
                .route("/fail-flow", post(Self::admin_fail_flow))
                .layer(middleware::from_fn(move |request: Request, next: Next| {
                    let admin_token = admin_token_for_middleware.clone();
                    async move {
                        Self::verify_admin_request(admin_token.as_deref(), request, next).await
                    }
                }))
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

    async fn admin_fail_flow(
        Extension(broker): Extension<Arc<FundingSyncBroker>>,
        Json(body): Json<FailFlowReq>,
    ) -> Response {
        let payload = match build_fail_flow_payload(&body) {
            Ok(payload) => payload,
            Err(err) => return err.into_response(),
        };

        info!(
            "admin_fail_flow accepted: kind={}, flow_id={}, reason_len={}",
            body.kind,
            body.flow_id,
            body.reason.len()
        );

        // Forward to coordinator. 202 = broker accepted; cleanup happens async.
        match broker.send(&FromServer::UserRequest(payload)).await {
            Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "result": "accepted" }))).into_response(),
            Err(err) => err.into_response(),
        }
    }

    async fn verify_admin_request(
        admin_token: Option<&str>,
        request: Request,
        next: Next,
    ) -> Response {
        if let Err(err) = verify_admin_auth(admin_token, request.headers()) {
            return err.into_response();
        }
        next.run(request).await
    }

    async fn apply_stream(
        Extension(broker): Extension<Arc<FundingSyncBroker>>,
        Json(body): Json<ApplyStreamReq>,
    ) -> impl IntoResponse {
        info!(
            "Received apply stream request for destination: {} with payload: {:?}",
            broker.destination, body
        );

        let payload = build_apply_stream_payload(&body);
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

/// Authorise an admin request. Returns `Unauthorized` either when no token is
/// configured (endpoint disabled) or when the bearer header doesn't match.
fn verify_admin_auth(configured_token: Option<&str>, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = configured_token.filter(|t| !t.is_empty()) else {
        return Err(ApiError::Unauthorized("admin endpoints disabled".into()));
    };
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if provided != Some(expected) {
        warn!("admin_fail_flow rejected: invalid or missing bearer token");
        return Err(ApiError::Unauthorized("invalid token".into()));
    }
    Ok(())
}

/// Validate the `FailFlow` body and re-emit it in the wire shape the coordinator
/// expects (`{"Admin":{"FailFlow":{...}}}`). Pure: no side effects.
fn build_fail_flow_payload(body: &FailFlowReq) -> Result<Value, ApiError> {
    const ALLOWED_KINDS: [&str; 4] = ["pegin", "pegout", "advance-funds", "committee-setup"];
    if !ALLOWED_KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::InvalidData(format!("kind must be one of {ALLOWED_KINDS:?}")));
    }
    if body.reason.trim().is_empty() || body.reason.len() > 512 {
        return Err(ApiError::InvalidData("reason must be 1..=512 chars".into()));
    }
    Ok(json!({
        "Admin": {
            "FailFlow": {
                "kind": body.kind,
                "flow_id": body.flow_id,
                "reason": body.reason,
            }
        }
    }))
}

/// Rebuild the only coordinator request shape accepted by `/member/apply-stream`.
fn build_apply_stream_payload(body: &ApplyStreamReq) -> Value {
    json!({ "ApplyToStream": &body.apply_to_stream })
}

/// Validates a 32-byte X-only public key with 0x prefix
fn is_valid_xonly_pubkey(key: &str) -> bool {
    key.len() == 66 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Validates a 33-byte compressed public key with 0x prefix
fn is_valid_compressed_pubkey(key: &str) -> bool {
    key.len() == 68 && key.starts_with("0x") && key[2..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn header_with_auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_str(value).expect("header"));
        h
    }

    #[test]
    fn verify_admin_auth_disabled_when_no_token_configured() {
        for token in [None, Some(""), Some("   ").map(|_| "")] {
            let headers = header_with_auth("Bearer whatever");
            let err = verify_admin_auth(token, &headers).expect_err("expected disabled");
            assert!(
                matches!(err, ApiError::Unauthorized(ref msg) if msg.contains("disabled")),
                "got {err:?}"
            );
        }
    }

    #[test]
    fn verify_admin_auth_rejects_missing_or_wrong_bearer() {
        // No header at all.
        let err = verify_admin_auth(Some("secret"), &HeaderMap::new())
            .expect_err("missing header should be rejected");
        assert!(matches!(err, ApiError::Unauthorized(_)));

        // Wrong scheme.
        let headers = header_with_auth("Basic secret");
        let err = verify_admin_auth(Some("secret"), &headers)
            .expect_err("non-Bearer scheme should be rejected");
        assert!(matches!(err, ApiError::Unauthorized(_)));

        // Right scheme, wrong token.
        let headers = header_with_auth("Bearer wrong");
        let err = verify_admin_auth(Some("secret"), &headers)
            .expect_err("mismatched token should be rejected");
        assert!(matches!(err, ApiError::Unauthorized(ref msg) if msg.contains("invalid")));
    }

    #[test]
    fn verify_admin_auth_accepts_matching_bearer() {
        let headers = header_with_auth("Bearer s3cret");
        verify_admin_auth(Some("s3cret"), &headers).expect("auth should succeed");
    }

    fn req(kind: &str, reason: &str) -> FailFlowReq {
        FailFlowReq { kind: kind.into(), flow_id: uuid::Uuid::new_v4(), reason: reason.into() }
    }

    #[test]
    fn build_fail_flow_payload_rejects_unknown_kind() {
        let err = build_fail_flow_payload(&req("frobnicate", "ok"))
            .expect_err("unknown kind should be rejected");
        assert!(matches!(err, ApiError::InvalidData(ref msg) if msg.contains("kind")));
    }

    #[test]
    fn build_fail_flow_payload_rejects_empty_reason() {
        let err = build_fail_flow_payload(&req("pegin", "   "))
            .expect_err("blank reason should be rejected");
        assert!(matches!(err, ApiError::InvalidData(ref msg) if msg.contains("reason")));
    }

    #[test]
    fn build_fail_flow_payload_rejects_oversized_reason() {
        let huge = "x".repeat(513);
        let err = build_fail_flow_payload(&req("pegin", &huge))
            .expect_err("reason >512 chars should be rejected");
        assert!(matches!(err, ApiError::InvalidData(ref msg) if msg.contains("reason")));
    }

    #[test]
    fn build_fail_flow_payload_emits_admin_failflow_shape() {
        let body = req("advance-funds", "manual cleanup");
        let payload = build_fail_flow_payload(&body).expect("valid payload");

        // Must round-trip into the coordinator's UserRequests::Admin(FailFlow{...}).
        // We assert the raw JSON shape so coordinator-side serde changes show up here.
        let admin = payload.get("Admin").expect("Admin key");
        let fail = admin.get("FailFlow").expect("FailFlow key");
        assert_eq!(fail.get("kind").and_then(Value::as_str), Some("advance-funds"));
        assert_eq!(
            fail.get("flow_id").and_then(Value::as_str),
            Some(body.flow_id.to_string()).as_deref()
        );
        assert_eq!(fail.get("reason").and_then(Value::as_str), Some("manual cleanup"));
    }

    #[test]
    fn build_fail_flow_payload_accepts_all_documented_kinds() {
        for kind in ["pegin", "pegout", "advance-funds", "committee-setup"] {
            build_fail_flow_payload(&req(kind, "x")).unwrap_or_else(|e| {
                panic!("kind {kind} should be valid, got {e:?}");
            });
        }
    }

    #[test]
    fn apply_stream_payload_rebuilds_only_apply_to_stream_shape() {
        let body: ApplyStreamReq = serde_json::from_value(json!({
            "ApplyToStream": {
                "stream_id": 1,
                "role": "Prover",
                "funding_utxo": { "value": 10 },
                "speed_up_utxo": { "value": 20 },
                "advance_funds": { "value": 30 }
            }
        }))
        .expect("valid apply-stream payload");

        let payload = build_apply_stream_payload(&body);

        assert!(payload.get("Admin").is_none());
        assert_eq!(payload["ApplyToStream"]["stream_id"], 1);
        assert_eq!(payload["ApplyToStream"]["role"], "Prover");
        assert_eq!(payload["ApplyToStream"]["funding_utxo"]["value"], 10);
    }

    #[test]
    fn apply_stream_payload_rejects_admin_shape() {
        let payload = json!({
            "Admin": {
                "FailFlow": {
                    "kind": "pegin",
                    "flow_id": Uuid::new_v4(),
                    "reason": "bypass"
                }
            }
        });

        serde_json::from_value::<ApplyStreamReq>(payload)
            .expect_err("admin request must not deserialize as apply-stream");
    }

    #[test]
    fn apply_stream_payload_rejects_extra_top_level_fields() {
        let payload = json!({
            "ApplyToStream": {
                "stream_id": 1,
                "role": "Verifier",
                "funding_utxo": { "value": 10 },
                "speed_up_utxo": { "value": 20 },
                "advance_funds": { "value": 30 }
            },
            "Admin": {
                "FailFlow": {
                    "kind": "pegin",
                    "flow_id": Uuid::new_v4(),
                    "reason": "bypass"
                }
            }
        });

        serde_json::from_value::<ApplyStreamReq>(payload)
            .expect_err("extra top-level fields must be rejected");
    }
}
