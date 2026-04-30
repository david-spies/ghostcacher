// ghostcacher-control-plane/src/admin.rs
// Admin API handlers for the GhostCacher Control Plane

use axum::{
    extract::{Path, State},
    response::Json,
    http::StatusCode,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::info;

use crate::AppState;
use crate::pod_registry::PodInfo;

pub async fn list_pods_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let pods = state.pods.list_pods().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "pods": pods, "count": pods.len() })))
}

pub async fn pod_heartbeat_handler(
    State(state): State<Arc<AppState>>,
    Path(pod_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let info: PodInfo = serde_json::from_value(body)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    state.pods.upsert_pod(&info).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "registered": true, "pod_id": pod_id })))
}

pub async fn list_cache_entries_handler(
    State(_state): State<Arc<AppState>>,
) -> Json<Value> {
    // Production: SCAN cache_map:* and return paginated results
    Json(json!({
        "entries": [],
        "note": "Use /gc/stats for aggregate metrics"
    }))
}

pub async fn flush_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let scope = body.get("scope")
        .and_then(|s| s.as_str())
        .unwrap_or("session");

    let flushed = state.eviction.flush(scope).await.unwrap_or(0);
    info!(%scope, flushed, "Cache flush completed");

    Json(json!({ "flushed": flushed, "scope": scope }))
}

pub async fn cluster_stats_handler(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let pods = state.pods.list_pods().await.unwrap_or_default();
    let avg_hbm = if pods.is_empty() {
        0.0
    } else {
        pods.iter().map(|p| p.hbm_util).sum::<f32>() / pods.len() as f32
    };

    Json(json!({
        "cluster": {
            "pod_count":       pods.len(),
            "avg_hbm_util":    avg_hbm,
            "total_kv_blocks": pods.iter().map(|p| p.kv_block_count).sum::<u32>(),
        },
        "version": "2.1.0"
    }))
}

pub async fn metrics_handler(
    State(_state): State<Arc<AppState>>,
) -> String {
    // Production: render Prometheus text format from registry
    "# GhostCacher Control Plane metrics\n".to_string()
}
