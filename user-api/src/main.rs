use anyhow::{Context, Result};
use common::config::CommonConfig;
use common::msg_broker::broker::BrokerServer;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use user_api::Server;

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[tokio::main]
async fn main() -> Result<()> {
    CommonConfig::init_logger(None, CARGO_PKG_NAME)?;

    info!("Starting user-api server");

    let shutdown_flag = ShutdownFlag::init();

    let broker_port = 5550;
    let broker_server = Arc::new(BrokerServer::new(broker_port));
    info!("Broker Server started on {broker_port}");

    let http_addr = SocketAddr::from(([0, 0, 0, 0], 5551));
    let listener = TcpListener::bind(http_addr)
        .await
        .context("Failed to bind to address")?;
    let server = Server::new(listener, broker_server.clone(), shutdown_flag.clone()).await;
    info!("Http Server started, listening on {http_addr}");
    if let Err(err) = server.start().await {
        error!("Server error: {}", err);
        return Err(err);
    }

    // this is required due to BlockServer creating its own runtime, and it cannot be dropped in an async context: Cannot drop a runtime in a context where blocking is not allowed
    tokio::spawn(async move {
        shutdown_flag.wait_for().await;

        tokio::task::spawn_blocking(move || {
            drop(broker_server); // <- drop in blocking context
        })
        .await
        .expect("failed to drop broker server");
    });

    info!("Server shutdown complete");

    Ok(())
}
