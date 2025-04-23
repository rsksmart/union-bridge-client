use crate::rsk_gateway::PegManagerErrors;
use crate::rsk_gateway::{RskContractsGateway, RskContractsGatewayApi};
use crate::types::{PegInAddressInput, RegisterPegInInput};
use alloy_provider::Provider;
use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use common::shutdown_flag::ShutdownFlag;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
}

impl Server {
    pub async fn new<P: Provider + 'static>(
        listener: TcpListener,
        rsk_contract_gateway: Arc<RskContractsGateway<P>>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let app = Router::new()
            .route("/pegin-address", post(Self::create_peg_in_address::<P>))
            .route("/register-pegin", post(Self::register_peg_in::<P>))
            .route("/accept-pegin", post(Self::accept_peg_in::<P>))
            .layer((
                // TraceLayer::new_for_http(), // TODO: enable when we change logging library to tracing
                TimeoutLayer::new(Duration::from_secs(10)),
                Extension(rsk_contract_gateway),
            ));

        Server {
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

    async fn create_peg_in_address<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGateway<P>>>,
        Json(payload): Json<PegInAddressInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.get_temporary_peg_in_address(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn register_peg_in<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGateway<P>>>,
        Json(payload): Json<RegisterPegInInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.register_peg_in_request(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn accept_peg_in<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGateway<P>>>,
        Json(payload): Json<RegisterPegInInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.accept_peg_in_request(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }
}

impl IntoResponse for PegManagerErrors {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            // bad request
            PegManagerErrors::InvalidAddress(msg)
            | PegManagerErrors::InvalidPublicKey(msg)
            | PegManagerErrors::InvalidValue(msg)
            | PegManagerErrors::InvalidBtcTxSpvProof(msg) => (StatusCode::BAD_REQUEST, msg),

            // not found
            PegManagerErrors::UnregisteredRequest(msg)
            | PegManagerErrors::PacketOutOfBound(msg)
            | PegManagerErrors::StreamNotFoundByDenomination(msg) => (StatusCode::NOT_FOUND, msg),

            // forbidden
            PegManagerErrors::AlreadyRegisteredAcceptPegIn(msg)
            | PegManagerErrors::AlreadyRegisteredPegIn(msg)
            | PegManagerErrors::AlreadyRegisteredPegInRequest(msg) => (StatusCode::FORBIDDEN, msg),

            // conflict
            PegManagerErrors::NoRevertError(msg)
            | PegManagerErrors::NotEnoughConfirmations(msg) => (StatusCode::CONFLICT, msg),

            // unauthorized
            PegManagerErrors::NotOwner(msg) => (StatusCode::UNAUTHORIZED, msg),

            // unhandled => internal server error
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
