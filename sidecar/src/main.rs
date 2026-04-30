// ghostcacher-sidecar/src/main.rs
// GhostCacher Sidecar — production-grade LLM request interceptor
// Sits in the same Kubernetes pod as the application, zero-latency local interception

mod config;
mod hasher;
mod interceptor;
mod metrics;
mod provider;
mod redis_client;
mod router;
mod types;

use anyhow::Result;
use axum::{Router, middleware};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::SidecarConfig;
use crate::interceptor::InterceptorState;
use crate::metrics::MetricsRegistry;
use crate::redis_client::RedisControlPlane;
use crate::router::build_router;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured JSON logging (OpenTelemetry-compatible)
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "ghostcacher_sidecar=debug,tower_http=info".into()
        }))
        .with(fmt::layer().json())
        .init();

    info!(version = "2.1.0", "GhostCacher sidecar starting");

    // Load config (env vars → config file → defaults)
    let cfg = Arc::new(SidecarConfig::load()?);
    info!(
        listen_addr = %cfg.listen_addr,
        upstream = %cfg.upstream_url,
        redis = %cfg.redis_url,
        "Configuration loaded"
    );

    // Initialize Redis control plane client
    let redis = Arc::new(RedisControlPlane::connect(&cfg.redis_url).await?);
    info!("Redis control plane connected");

    // Initialize Prometheus metrics registry
    let metrics = Arc::new(MetricsRegistry::new()?);

    // Build shared interceptor state
    let state = Arc::new(InterceptorState::new(cfg.clone(), redis, metrics.clone()));

    // Build axum application
    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(
            cfg.request_timeout_secs,
        )))
        .layer(CorsLayer::permissive());

    // Metrics endpoint on a separate port
    let metrics_app = Router::new().route(
        "/metrics",
        axum::routing::get({
            let m = metrics.clone();
            move || async move { m.render() }
        }),
    );

    let listen: SocketAddr = cfg.listen_addr.parse()?;
    let metrics_listen: SocketAddr = cfg.metrics_addr.parse()?;

    info!(%listen, "Sidecar proxy listening");
    info!(%metrics_listen, "Prometheus metrics endpoint listening");

    // Run both servers concurrently
    let (proxy_result, metrics_result) = tokio::join!(
        async {
            let listener = TcpListener::bind(listen).await?;
            axum::serve(listener, app).await.map_err(anyhow::Error::from)
        },
        async {
            let listener = TcpListener::bind(metrics_listen).await?;
            axum::serve(listener, metrics_app)
                .await
                .map_err(anyhow::Error::from)
        }
    );

    proxy_result?;
    metrics_result?;
    Ok(())
}
