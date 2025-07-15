use crate::{
    rsk_gateway::{DomainErrors, RskContractsGateway, RskContractsGatewayApi},
    types::{
        AddMemberNonceInput, AddMemberSignatureInput, PegInAddressInput, RegisterPegInInput,
        RegisterPegOutInput,
    },
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
            .route("/pegin-address", post(Self::create_peg_in_address::<P>))
            .route("/register-pegin", post(Self::register_peg_in::<P>))
            .route("/accept-pegin", post(Self::accept_peg_in::<P>))
            .route("/register-pegout", post(Self::register_peg_out::<P>))
            .route("/add-member-nonce", post(Self::add_member_nonce::<P>))
            .route(
                "/add-member-signature",
                post(Self::add_member_signature::<P>),
            )
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

    #[allow(unused)]
    async fn add_member_nonce<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<AddMemberNonceInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.add_member_nonce(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    #[allow(unused)]
    async fn add_member_signature<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<AddMemberSignatureInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.add_member_signature(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn create_peg_in_address<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<PegInAddressInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.get_temporary_peg_in_address(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn register_peg_in<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RegisterPegInInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.register_peg_in_request(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn accept_peg_in<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RegisterPegInInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.accept_peg_in_request(payload).await {
            Ok(data) => (StatusCode::OK, Json(json!(data))).into_response(),
            Err(e) => e.into_response(),
        }
    }

    async fn register_peg_out<P: Provider>(
        Extension(rsk_gateway): Extension<RskContractsGateway<P>>,
        Json(payload): Json<RegisterPegOutInput>,
    ) -> impl IntoResponse {
        match rsk_gateway.register_peg_out_request(payload).await {
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
