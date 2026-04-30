// ghostcacher-kv-relay/src/main.rs
// GhostCacher KV-Relay Service
//
// Handles cross-node KV tensor transfer for self-hosted GPU clusters.
// When a request arrives at pod-B for a prefix warmed on pod-A,
// the relay fetches the KV blocks from pod-A and injects them into
// pod-B's vLLM/SGLang RadixAttention cache, bypassing the prefill phase.
//
// Transport: gRPC over RDMA (via SmartNIC / Mellanox BlueField-3)
//
// This service runs as a DaemonSet on every GPU node.
// It exposes:
//   - gRPC server:  :50051  (tensor receive endpoint for incoming transfers)
//   - gRPC client:  outbound connections to peer nodes on demand
//   - HTTP health:  :50052  /healthz

mod relay;
mod config;
mod transfer;

use anyhow::Result;
use axum::{Router, routing::get};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::RelayConfig;
use crate::relay::KvRelayService;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "ghostcacher_kv_relay=debug".into()))
        .with(fmt::layer().json())
        .init();

    info!(version = "2.1.0", "GhostCacher KV-Relay starting");

    let cfg = Arc::new(RelayConfig::load()?);
    let relay_svc = Arc::new(KvRelayService::new(cfg.clone()).await?);

    // Spawn gRPC relay server (receives incoming KV tensor payloads)
    let relay_clone = relay_svc.clone();
    let grpc_addr: SocketAddr = cfg.grpc_addr.parse()?;
    tokio::spawn(async move {
        info!(%grpc_addr, "KV-Relay gRPC server listening");
        relay_clone.serve(grpc_addr).await.expect("gRPC server failed");
    });

    // HTTP health server
    let health_app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz",  get(|| async { "ok" }));

    let http_addr: SocketAddr = cfg.http_addr.parse()?;
    info!(%http_addr, "KV-Relay health server listening");
    let listener = TcpListener::bind(http_addr).await?;
    axum::serve(listener, health_app).await?;

    Ok(())
}
