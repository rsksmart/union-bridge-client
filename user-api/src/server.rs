use anyhow::{Context, Result};
use axum::routing::post;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Extension, Json, Router};
use common::msg_broker::broker::{BrokerServer, BrokerServerApi};
use common::msg_broker::types::FromServer;
use common::shutdown_flag::ShutdownFlag;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::timeout::TimeoutLayer;

pub struct Server {
    listener: TcpListener,
    app: Router,
    shutdown_flag: ShutdownFlag,
    broker_server: Arc<BrokerServer>,
}

impl Server {
    pub async fn new(
        listener: TcpListener,
        broker_server: Arc<BrokerServer>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let app = Router::new()
            .route("/health", get(Self::health_check))
            .route("/apply-stream", post(Self::apply_stream))
            .layer((
                TimeoutLayer::new(Duration::from_secs(10)),
                Extension(broker_server.clone()),
            ));

        Self {
            listener,
            app,
            shutdown_flag,
            broker_server,
        }
    }

    pub async fn start(self) -> Result<()> {
        // Set up a shutdown handler to properly close the broker server
        let broker_server = self.broker_server.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        tokio::spawn(async move {
            shutdown_flag.wait_for().await;
            // Close the broker server before the runtime is dropped
            if let Ok(mut broker) = Arc::try_unwrap(broker_server) {
                broker.close();
            }
        });

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
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        let res = broker.send(&FromServer::UserApplyStream(payload), 333); // TODO(iago) hardcoded for now
        match res {
            Ok(_) => (StatusCode::OK, Json(json!({ "result": "ok" }))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ),
        }
    }
}
