use crate::contracts::peg_manager::{PegManagerErrors, PegManagerGatewayApi};
use crate::rsk_gateway::{RskContractsGateway, RskContractsGatewayApi};
use crate::use_cases::get_temporary_peg_in_address::{PegInAddressInput, PegInAddressOutput};
use crate::use_cases::register_peg_in_request::RegisterPegInInput;
use alloy_provider::Provider;
use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use common::shutdown_flag::ShutdownFlag;
use log::error;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
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
    ) -> Result<Json<PegInAddressOutput>, ApiError> {
        match rsk_gateway
            .get_peg_manager()
            .get_temporary_peg_in_address(payload)
            .await
        {
            Ok(address) => Ok(Json(address)),
            Err(e) => match e {
                PegManagerErrors::InvalidPublicKey
                | PegManagerErrors::InvalidAddress
                | PegManagerErrors::InvalidValue => Err(ApiError::BadRequest(e.to_string())),
                PegManagerErrors::StreamNotFoundByDenomination => {
                    Err(ApiError::NotFound(e.to_string()))
                }
                _ => Err(ApiError::BadRequest(e.to_string())),
            },
        }
    }

    async fn register_peg_in<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGateway<P>>>,
        Json(payload): Json<RegisterPegInInput>,
    ) -> Result<(), ApiError> {
        match rsk_gateway
            .get_peg_manager()
            .register_peg_in_request(payload)
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                // TODO(iago) properly map errors
                Err(ApiError::InternalServerError)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Internal server error")]
    InternalServerError,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.to_string()),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.to_string()),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::INTERNAL_SERVER_ERROR.to_string(),
            ),
        };

        let body = Json(json!({ "error": message }));
        (status, body).into_response()
    }
}
