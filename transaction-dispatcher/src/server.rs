use crate::contracts::peg_manager::{
    PegManagerErrors, PeginAddressInput, PeginAddressOutput, RegisterPeginInput,
};
use crate::rsk_gateway::{RskContractsGateway, RskContractsGatewayAlloy};
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
    pub async fn new<P: Provider + 'static, T: RskContractsGateway<P> + Send + Sync + 'static>(
        listener: TcpListener,
        rsk_contract_gateway: Arc<T>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let app = Router::new()
            .route("/pegin-address", post(Self::create_pegin_address::<P>))
            .route("/register-pegin", post(Self::register_pegin::<P>))
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

    async fn create_pegin_address<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGatewayAlloy<P>>>,
        Json(payload): Json<PeginAddressInput>,
    ) -> Result<Json<PeginAddressOutput>, ApiError> {
        match rsk_gateway
            .get_peg_manager()
            .get_temporary_pegin_address(payload)
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

    async fn register_pegin<P: Provider>(
        Extension(rsk_gateway): Extension<Arc<RskContractsGatewayAlloy<P>>>,
        Json(payload): Json<RegisterPeginInput>,
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
