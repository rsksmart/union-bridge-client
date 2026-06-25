//! Prometheus metrics infrastructure shared by all union-bridge-client binaries.
//!
//! Each binary calls [`init_prometheus_recorder`] once at startup to install a
//! global `metrics` recorder, then spawns [`serve_metrics`] on its tokio runtime
//! to expose `/metrics` on its configured bind address. Business code emits
//! samples through the `metrics` facade (`metrics::counter!`, etc.) without
//! depending on the exporter directly — the same decoupled pattern as `tracing`.

use std::net::SocketAddr;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tracing::{error, info, instrument};

use crate::shutdown_flag::ShutdownFlag;

/// Per-service monitoring configuration loaded from TOML.
///
/// `enabled = false` lets operators turn the endpoint off without removing the
/// section entirely. Defaults to `true` so a missing flag opts in.
#[derive(Debug, Clone, Deserialize)]
pub struct MonitoringConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub bind_addr: SocketAddr,
}

const fn default_enabled() -> bool {
    true
}

/// Install the global Prometheus recorder.
///
/// `service` is attached as a global label so all samples emitted by this
/// process are distinguishable in Prometheus when multiple operator services
/// scrape into the same instance.
///
/// # Errors
///
/// Returns an error if a global recorder was already installed (typically only
/// happens when called twice in the same process, e.g. in tests).
pub fn init_prometheus_recorder(service: &str) -> Result<PrometheusHandle> {
    let handle = PrometheusBuilder::new()
        .add_global_label("service", service)
        .install_recorder()
        .context("install prometheus recorder")?;
    // Seed a build-info sample so the service is visible in Prometheus from
    // startup, before it emits any activity-driven metric. Otherwise idle
    // services (user-api with no requests, log-indexer with no logs) export
    // zero samples and look unreachable in any view keyed on the `service`
    // label. `version` is the workspace build version (all crates share it).
    metrics::gauge!("union_build_info", "version" => env!("CARGO_PKG_VERSION")).set(1.0);
    Ok(handle)
}

/// Build the axum router that serves `GET /metrics`.
///
/// Exposed separately from [`serve_metrics`] as a reusable router so a binary
/// that already owns an axum server can embed `/metrics` instead of opening a
/// second listener.
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let handle = handle.clone();
            async move { (StatusCode::OK, handle.render()) }
        }),
    )
}

/// Run a dedicated HTTP server exposing `/metrics` on an already-bound listener.
///
/// The caller binds the listener (see [`install_and_spawn`]) so a bind failure
/// surfaces at startup and aborts it, rather than being logged-and-ignored on
/// this serving task. The server shuts down gracefully when `shutdown_flag` is
/// set. Use this from binaries that don't already own an axum server
/// (coordinator, indexers).
///
/// # Errors
///
/// Returns an error if the listener cannot be adopted by the tokio runtime or
/// the underlying axum server errors out.
#[instrument(skip_all)]
pub async fn serve_metrics(
    listener: std::net::TcpListener,
    handle: PrometheusHandle,
    shutdown_flag: ShutdownFlag,
) -> Result<()> {
    let listener = TcpListener::from_std(listener).context("adopt prometheus /metrics listener")?;
    let addr = listener.local_addr().context("read prometheus /metrics listener address")?;
    info!("Prometheus /metrics endpoint listening on {addr}");

    axum::serve(listener, metrics_router(handle))
        .with_graceful_shutdown(shutdown_flag.wait_for())
        .await
        .context("prometheus /metrics server terminated with error")
}

/// Convenience for sync binaries (coordinator, indexers): install the global
/// recorder and spawn a dedicated OS thread running a single-threaded tokio
/// runtime that serves `/metrics` until the shutdown flag is set.
///
/// `Ok(None)` means monitoring was disabled by configuration; the caller can
/// drop it and emit metrics anyway (they will simply not be scraped).
///
/// # Errors
///
/// Returns an error if the recorder cannot be installed, the `/metrics` address
/// cannot be bound, or the OS thread cannot be spawned.
pub fn install_and_spawn(
    config: &MonitoringConfig,
    service: &'static str,
    shutdown_flag: ShutdownFlag,
) -> Result<Option<JoinHandle<()>>> {
    if !config.enabled {
        info!("Prometheus metrics disabled for {service}");
        return Ok(None);
    }
    let handle = init_prometheus_recorder(service)?;
    // Bind on the calling thread so a port conflict or bad bind_addr fails
    // startup here (propagated via `?`) instead of being logged-and-ignored on
    // the detached metrics thread. The bound listener is handed to the runtime.
    let listener = std::net::TcpListener::bind(config.bind_addr)
        .with_context(|| format!("bind prometheus /metrics endpoint on {}", config.bind_addr))?;
    listener.set_nonblocking(true).context("set prometheus /metrics listener non-blocking")?;
    let join = thread::Builder::new()
        .name(format!("{service}-metrics"))
        .spawn(move || {
            let rt = match Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(err) => {
                    error!("Failed to build metrics tokio runtime: {err}");
                    return;
                }
            };
            if let Err(err) = rt.block_on(serve_metrics(listener, handle, shutdown_flag)) {
                error!("Prometheus /metrics server terminated with error: {err:?}");
            }
        })
        .context("spawn prometheus /metrics thread")?;
    Ok(Some(join))
}

/// Axum middleware emitting RED metrics (`http_requests_total` counter and
/// `http_request_duration_seconds` histogram).
///
/// Uses the matched route template as the `path` label so per-request URI
/// parameters cannot blow up label cardinality. Requests that don't match any
/// route are reported as `unknown`.
pub async fn track_http_metrics(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path =
        req.extensions().get::<MatchedPath>().map_or("unknown", MatchedPath::as_str).to_owned();
    let start = Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16().to_string();
    let elapsed = start.elapsed().as_secs_f64();

    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "path" => path.clone(),
        "status" => status,
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "path" => path,
    )
    .record(elapsed);

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitoring_config_parses_with_explicit_enabled() {
        let cfg: MonitoringConfig = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "bind_addr": "127.0.0.1:9201",
        }))
        .expect("parse monitoring config");
        assert!(!cfg.enabled);
        assert_eq!(cfg.bind_addr, "127.0.0.1:9201".parse().unwrap());
    }

    #[test]
    fn monitoring_config_defaults_enabled_to_true() {
        let cfg: MonitoringConfig = serde_json::from_value(serde_json::json!({
            "bind_addr": "0.0.0.0:9201",
        }))
        .expect("parse monitoring config");
        assert!(cfg.enabled);
    }
}
