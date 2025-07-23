use crate::rsk_gateway::{DomainErrors, RskContractsGateway, RskContractsGatewayApi};
use crate::types::{
    AddMemberNonceInput, AddMemberSignatureInput, PeginAddressInput, RequestPeginInput,
    RequestPegoutInput,
};
use alloy_provider::Provider;
use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use common::shutdown_flag::ShutdownFlag;
use log::debug;
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
}

impl Server {
    pub async fn new<P: Provider + Clone + 'static>(
        listener: TcpListener,
        rsk_contract_gateway: RskContractsGateway<P>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let app = Router::new()
            .route("/pegin-address", post(Self::create_pegin_address::<P>))
            .route("/register-pegin", post(Self::request_pegin::<P>))
            .route("/accept-pegin", post(Self::accept_pegin::<P>))
            .route("/request-pegout", post(Self::request_pegout::<P>))
            .layer((
                // TraceLayer::new_for_http(), // TODO: enable when we change logging library to tracing
                TimeoutLayer::new(Duration::from_secs(10)),
                Extension(rsk_contract_gateway),
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

    async fn add_member_nonce<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<AddMemberNonceInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.add_member_nonce(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn add_member_signature<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<AddMemberSignatureInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.add_member_signature(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn request_pegout<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RequestPegoutInput>,
    ) -> impl IntoResponse {
        debug!("Handling pegout request: {:?}", payload);
        match rsk_gateway.request_pegout(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn create_pegin_address<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<PeginAddressInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.get_temporary_pegin_address(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn request_pegin<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RequestPeginInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.request_pegin(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn accept_pegin<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RequestPeginInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.accept_pegin(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }
}

impl IntoResponse for DomainErrors {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            // bad request
            DomainErrors::InvalidAddress(msg)
            | DomainErrors::InvalidCompressedPubKey(msg)
            | DomainErrors::InvalidValue(msg)
            | DomainErrors::InvalidBtcTxSpvProof(msg) => (StatusCode::BAD_REQUEST, msg),

            // not found
            DomainErrors::PacketOutOfBound(msg)
            | DomainErrors::StreamNotFoundByDenomination(msg) => (StatusCode::NOT_FOUND, msg),

            // forbidden
            DomainErrors::PeginAlreadyRequested(msg) | DomainErrors::PeginAlreadyAccepted(msg) => {
                (StatusCode::FORBIDDEN, msg)
            }

            // conflict
            DomainErrors::NoRevertError(msg) | DomainErrors::NotEnoughConfirmations(msg) => {
                (StatusCode::CONFLICT, msg)
            }

            // unauthorized
            DomainErrors::NotOwner(msg) => (StatusCode::UNAUTHORIZED, msg),

            // unhandled => internal server error
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
