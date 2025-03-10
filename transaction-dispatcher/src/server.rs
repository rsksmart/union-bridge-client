use crate::rsk_connector::RskContractsGateway;
use crate::types::{PeginAddressInput, PeginAddressOutput};
use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};
use std::sync::Arc;

pub struct Server {
    listener: tokio::net::TcpListener,
    app: Router,
}

impl Server {
    pub async fn new(
        listener: tokio::net::TcpListener,
        rsk_contract_gateway: Arc<RskContractsGateway>,
    ) -> Self {
        let app = Router::new()
            .route("/pegin-address", post(Self::create_pegin_address))
            .layer(Extension(rsk_contract_gateway));

        Server { listener, app }
    }

    pub async fn start(self) -> Result<()> {
        axum::serve(self.listener, self.app)
            .await
            .context("Error starting server")
    }

    async fn create_pegin_address(
        Extension(rsk_gateway): Extension<Arc<RskContractsGateway>>,
        Json(payload): Json<PeginAddressInput>,
    ) -> (StatusCode, Json<PeginAddressOutput>) {
        match rsk_gateway.get_temporary_pegin_address(payload).await {
            Ok(address) => (StatusCode::CREATED, Json(address)),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(PeginAddressOutput {
                    address: e.to_string(),
                }),
            ),
        }
    }
}
