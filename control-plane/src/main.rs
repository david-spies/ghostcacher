// ghostcacher-control-plane/src/main.rs
// GhostCacher Control Plane
//
// Responsibilities:
//   1. Pod Registry — maintains the live map of GPU pods and their HBM utilization
//   2. Eviction Orchestrator — runs cost-weighted TTL eviction on a schedule
//   3. Cluster Health — exposes aggregate cluster metrics to Prometheus
//   4. Admin API — flush, inspect, and reconfigure the distributed cache

mod eviction;
mod pod_registry;
mod admin;
mod config;

use anyhow::Result;
use axum::{Router, routing::{get, post, delete}};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::ControlPlaneConfig;
use crate::eviction::EvictionEngine;
use crate::pod_registry::PodRegistry;
use crate::admin::{
    list_pods_handler, pod_heartbeat_handler,
    list_cache_entries_handler, flush_handler,
    cluster_stats_handler, metrics_handler,
};

pub struct AppState {
    pub cfg:      Arc<ControlPlaneConfig>,
    pub pods:     Arc<PodRegistry>,
    pub eviction: Arc<EvictionEngine>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "ghostcacher_control_plane=debug".into()))
        .with(fmt::layer().json())
        .init();

    info!(version = "2.1.0", "GhostCacher Control Plane starting");

    let cfg = Arc::new(ControlPlaneConfig::load()?);
    let pods = Arc::new(PodRegistry::new(cfg.redis_url.clone()).await?);
    let eviction = Arc::new(EvictionEngine::new(pods.clone(), cfg.clone()));

    // Spawn background eviction loop
    let eviction_clone = eviction.clone();
    tokio::spawn(async move {
        eviction_clone.run_loop().await;
    });

    // Spawn pod health-check loop
    let pods_clone = pods.clone();
    tokio::spawn(async move {
        pods_clone.health_check_loop().await;
    });

    let state = Arc::new(AppState { cfg: cfg.clone(), pods, eviction });

    let app = Router::new()
        // Pod registry
        .route("/gc/pods",                   get(list_pods_handler))
        .route("/gc/pods/:pod_id/heartbeat", post(pod_heartbeat_handler))
        // Cache management
        .route("/gc/cache",                  get(list_cache_entries_handler))
        .route("/gc/cache/flush",            post(flush_handler))
        .route("/gc/cache/:hash",            delete(flush_handler))
        // Cluster stats + Prometheus
        .route("/gc/stats",                  get(cluster_stats_handler))
        .route("/metrics",                   get(metrics_handler))
        // Health
        .route("/healthz", get(|| async { "ok" }))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = cfg.listen_addr.parse()?;
    info!(%addr, "Control plane listening");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
