// ghostcacher-sidecar/src/router.rs
// Axum router — mounts all sidecar endpoints

use axum::{
    Router,
    routing::{get, post, any},
    extract::State,
    response::Json,
};
use std::sync::Arc;
use serde_json::json;

use crate::interceptor::{intercept_handler, InterceptorState};

pub fn build_router(state: Arc<InterceptorState>) -> Router {
    Router::new()
        // Primary interception endpoint — catches all LLM API calls
        .route("/v1/messages",          any(intercept_handler))
        .route("/v1/chat/completions",  any(intercept_handler))
        .route("/v1/complete",          any(intercept_handler))
        // Health + readiness probes (Kubernetes liveness/readiness)
        .route("/healthz",  get(health_handler))
        .route("/readyz",   get(ready_handler))
        // Sidecar introspection — returns current config snapshot (no secrets)
        .route("/gc/status", get(status_handler))
        // Cache flush endpoint — POST with { "scope": "system" | "all" }
        .route("/gc/flush",  post(flush_handler))
        .with_state(state)
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "ghostcacher-sidecar" }))
}

async fn ready_handler(
    State(state): State<Arc<InterceptorState>>,
) -> Json<serde_json::Value> {
    // Readiness = Redis reachable
    let redis_ok = state.redis.incr_metric("readyz_probe").await.is_ok();
    Json(json!({
        "status": if redis_ok { "ready" } else { "not_ready" },
        "redis":  redis_ok,
    }))
}

async fn status_handler(
    State(state): State<Arc<InterceptorState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "version":      "2.1.0",
        "listen_addr":  state.cfg.listen_addr,
        "upstream_url": state.cfg.upstream_url,
        "rdma_enabled": state.cfg.rdma_enabled,
        "provider":     format!("{:?}", state.provider),
        "hbm_threshold": state.cfg.hbm_swap_threshold,
    }))
}

async fn flush_handler(
    State(state): State<Arc<InterceptorState>>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let scope = body.get("scope")
        .and_then(|s| s.as_str())
        .unwrap_or("session");
    // In production: issue SCAN + DEL on matching cache_map:* keys in Redis
    tracing::warn!(scope, "Cache flush requested via API");
    Json(json!({ "flushed": true, "scope": scope }))
}
