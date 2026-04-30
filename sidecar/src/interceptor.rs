// ghostcacher-sidecar/src/interceptor.rs
// GhostCacher Interceptor — core request interception, hash, route, inject pipeline

use anyhow::Result;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode},
    response::IntoResponse,
};
use bytes::Bytes;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::{
    config::SidecarConfig,
    hasher::hash_prompt_blocks,
    metrics::MetricsRegistry,
    provider::ProviderAdapter,
    redis_client::RedisControlPlane,
    types::{
        BlockKind, CacheEntry, CacheLookupResult, LLMProvider, PromptBlock,
        RoutingDecision, TtlPolicy,
    },
};

/// Shared state injected into every Axum handler via `State<Arc<InterceptorState>>`
pub struct InterceptorState {
    pub cfg:      Arc<SidecarConfig>,
    pub redis:    Arc<RedisControlPlane>,
    pub metrics:  Arc<MetricsRegistry>,
    pub provider: LLMProvider,
}

impl InterceptorState {
    pub fn new(
        cfg:     Arc<SidecarConfig>,
        redis:   Arc<RedisControlPlane>,
        metrics: Arc<MetricsRegistry>,
    ) -> Self {
        let provider = LLMProvider::from_upstream_url(&cfg.upstream_url);
        Self { cfg, redis, metrics, provider }
    }
}

/// Main interception handler — called for every proxied LLM request.
///
/// Pipeline:
///   1. Parse body → extract GhostCacher prompt blocks
///   2. Hash stable blocks → derive prefix hash
///   3. Redis lookup → Hit / Partial Hit / Miss
///   4. Acquire Ghost-Lock on Miss (thundering herd protection)
///   5. Inject provider-specific cache headers
///   6. Forward to upstream pod, stream response back
///   7. On Miss: schedule async cache write after first token received
///   8. Emit Prometheus metrics
#[instrument(skip_all, fields(request_id))]
pub async fn intercept_handler(
    State(state): State<Arc<InterceptorState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4();
    tracing::Span::current().record("request_id", request_id.to_string());

    let t_start = unix_now_ms();

    // Collect full request body (bounded to 50MB for safety)
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
        Ok(b)  => b,
        Err(e) => {
            error!(%e, "Failed to read request body");
            return (StatusCode::BAD_REQUEST, "Body read error").into_response();
        }
    };

    // Parse and extract prompt blocks from the request body
    let blocks = match extract_prompt_blocks(&state, &body_bytes) {
        Ok(b)  => b,
        Err(e) => {
            warn!(%e, "Prompt extraction failed — proxying raw (no caching)");
            // Fall through: proxy without caching
            return forward_raw(&state, parts.headers, body_bytes, None).await;
        }
    };

    // Compute hierarchical hash
    let hash_result = hash_prompt_blocks(&blocks);
    debug!(
        prefix = %hash_result.prefix,
        h_sys  = ?hash_result.h_sys,
        h_doc  = ?hash_result.h_doc,
        tokens = hash_result.prefix_token_estimate,
        "Prompt hashed"
    );

    // Redis lookup
    let lookup = match state.redis.lookup(&hash_result.prefix).await {
        Ok(Some(entry)) => {
            state.metrics.record_hit(hash_result.prefix_token_estimate);
            let _ = state.redis.incr_metric("hits").await;

            let ttft_saved = estimate_ttft_savings(hash_result.prefix_token_estimate);
            info!(
                prefix = %hash_result.prefix,
                pod    = %entry.pod_ip,
                saved_ms = ttft_saved,
                "Cache HIT — routing to warmed pod"
            );

            CacheLookupResult::Hit {
                pod_ip:       entry.pod_ip.clone(),
                pod_id:       entry.pod_id.clone(),
                blocks_reused: entry.block_count,
            }
        }
        Ok(None) => {
            // Check if another pod is currently warming this prefix (Ghost-Lock)
            let lock_acquired = state.redis
                .try_acquire_ghost_lock(&hash_result.prefix)
                .await
                .unwrap_or(false);

            if !lock_acquired {
                // Another pod is warming — wait and retry
                info!(prefix = %hash_result.prefix, "Ghost-Lock contended — awaiting warm");
                if let Ok(Some(entry)) = state.redis
                    .await_ghost_lock_release(&hash_result.prefix)
                    .await
                {
                    state.metrics.record_hit(hash_result.prefix_token_estimate);
                    CacheLookupResult::Hit {
                        pod_ip:        entry.pod_ip,
                        pod_id:        entry.pod_id,
                        blocks_reused: entry.block_count,
                    }
                } else {
                    cold_miss_target(&state).await
                }
            } else {
                // We own the lock — this pod will warm the cache
                state.metrics.record_miss();
                let _ = state.redis.incr_metric("misses").await;
                info!(prefix = %hash_result.prefix, "Cache MISS — warming prefix");
                cold_miss_target(&state).await
            }
        }
        Err(e) => {
            warn!(%e, "Redis lookup error — proxying without cache");
            cold_miss_target(&state).await
        }
    };

    // Adapt headers for the target provider + inject cache_control breakpoint
    let adapted_body = ProviderAdapter::inject_cache_headers(
        &state.provider,
        &body_bytes,
        &blocks,
    );

    let pod_url = match &lookup {
        CacheLookupResult::Hit       { pod_ip, .. } => pod_url(pod_ip, &state.cfg),
        CacheLookupResult::PartialHit{ pod_ip, .. } => pod_url(pod_ip, &state.cfg),
        CacheLookupResult::Miss      { pod_ip, .. } => pod_url(pod_ip, &state.cfg),
    };

    // Schedule async cache write on miss
    if let CacheLookupResult::Miss { ref pod_ip, ref pod_id } = lookup {
        let redis     = state.redis.clone();
        let prefix    = hash_result.prefix.clone();
        let pod_ip_c  = pod_ip.clone();
        let pod_id_c  = pod_id.clone();
        let block_count = blocks.iter().filter(|b| b.kind.is_cacheable()).count() as u32;
        let tokens    = hash_result.prefix_token_estimate;
        let policy    = infer_ttl_policy(&blocks);

        tokio::spawn(async move {
            // Small delay — wait for first token to be generated (KV materialized)
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

            let now = unix_now();
            let entry = CacheEntry {
                prefix_hash: prefix.clone(),
                pod_ip:      pod_ip_c,
                pod_id:      pod_id_c.clone(),
                block_count,
                token_count: tokens,
                hit_count:   0,
                created_at:  now,
                last_hit_at: now,
                ttl_policy:  policy,
            };

            if let Err(e) = redis.write(&entry).await {
                warn!(%e, "Cache write failed");
            }

            let _ = redis.release_ghost_lock(&prefix).await;
        });
    }

    let elapsed = unix_now_ms() - t_start;
    debug!(elapsed_ms = elapsed, "Interception pipeline complete");

    forward_raw(&state, parts.headers, adapted_body, Some(pod_url)).await
}

/// Proxy the request to `url` (or the default upstream), stream the response back.
async fn forward_raw(
    state:    &InterceptorState,
    headers:  HeaderMap,
    body:     Bytes,
    url:      Option<String>,
) -> Response<Body> {
    let target = url.unwrap_or_else(|| state.cfg.upstream_url.clone());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(state.cfg.request_timeout_secs))
        .build()
        .expect("reqwest client build failed");

    let mut req_builder = client.post(&target).body(body);
    for (name, val) in &headers {
        if name != "host" {
            req_builder = req_builder.header(name, val);
        }
    }

    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let resp_body = resp.bytes().await.unwrap_or_default();

            let mut response = Response::new(Body::from(resp_body));
            *response.status_mut() = status;
            for (k, v) in &resp_headers {
                response.headers_mut().insert(k, v.clone());
            }
            response
        }
        Err(e) => {
            error!(%e, "Upstream forwarding error");
            (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e)).into_response()
        }
    }
}

/// Parse the request body into structured PromptBlocks.
/// Supports both GhostCacher's native structured format and
/// standard OpenAI/Anthropic message formats.
fn extract_prompt_blocks(
    state: &InterceptorState,
    body:  &[u8],
) -> Result<Vec<PromptBlock>> {
    let json: serde_json::Value = serde_json::from_slice(body)?;

    // GhostCacher native format: { "gc_blocks": [...] }
    if let Some(blocks_val) = json.get("gc_blocks") {
        let blocks: Vec<PromptBlock> = serde_json::from_value(blocks_val.clone())?;
        return Ok(blocks);
    }

    // Anthropic format: { "system": "...", "messages": [...] }
    if matches!(state.provider, LLMProvider::Anthropic) {
        return extract_anthropic_blocks(&json);
    }

    // OpenAI format: { "messages": [{"role": "system", ...}, ...] }
    extract_openai_blocks(&json)
}

fn extract_anthropic_blocks(json: &serde_json::Value) -> Result<Vec<PromptBlock>> {
    let mut blocks = Vec::new();

    if let Some(sys) = json.get("system").and_then(|s| s.as_str()) {
        blocks.push(PromptBlock {
            kind: BlockKind::System, content: sys.to_string(), hash: None,
        });
    }

    if let Some(tools) = json.get("tools") {
        blocks.push(PromptBlock {
            kind: BlockKind::Tools,
            content: serde_json::to_string(tools)?,
            hash: None,
        });
    }

    if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
        let last = messages.last();
        let prior = if messages.len() > 1 { &messages[..messages.len()-1] } else { &[] };

        // Prior messages as document context (cache-eligible)
        if !prior.is_empty() {
            blocks.push(PromptBlock {
                kind: BlockKind::Document,
                content: serde_json::to_string(prior)?,
                hash: None,
            });
        }

        // Last user message is volatile
        if let Some(last_msg) = last {
            blocks.push(PromptBlock {
                kind: BlockKind::User,
                content: serde_json::to_string(last_msg)?,
                hash: None,
            });
        }
    }

    Ok(blocks)
}

fn extract_openai_blocks(json: &serde_json::Value) -> Result<Vec<PromptBlock>> {
    let mut blocks = Vec::new();
    let messages = json.get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    let mut history = Vec::new();
    for msg in &messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" => blocks.push(PromptBlock {
                kind: BlockKind::System,
                content: serde_json::to_string(msg)?,
                hash: None,
            }),
            "tool" => blocks.push(PromptBlock {
                kind: BlockKind::Tools,
                content: serde_json::to_string(msg)?,
                hash: None,
            }),
            "user" | "assistant" => history.push(msg.clone()),
            _ => {}
        }
    }

    // All but last history message → Document block
    if history.len() > 1 {
        let doc = history[..history.len()-1].to_vec();
        blocks.push(PromptBlock {
            kind: BlockKind::Document,
            content: serde_json::to_string(&doc)?,
            hash: None,
        });
    }
    if let Some(last) = history.last() {
        blocks.push(PromptBlock {
            kind: BlockKind::User,
            content: serde_json::to_string(last)?,
            hash: None,
        });
    }

    Ok(blocks)
}

/// Infer the TTL policy from the block types present.
/// System-only → Infinite. Has docs → Fixed. Has session → Sliding.
fn infer_ttl_policy(blocks: &[PromptBlock]) -> TtlPolicy {
    let has_doc = blocks.iter().any(|b| matches!(b.kind, BlockKind::Document));
    let sys_only = blocks.iter().all(|b| {
        matches!(b.kind, BlockKind::System | BlockKind::Tools | BlockKind::User)
    });

    if sys_only { TtlPolicy::Infinite }
    else if has_doc { TtlPolicy::Fixed { secs: 4 * 3600 } }
    else { TtlPolicy::Sliding { window_secs: 3600 } }
}

async fn cold_miss_target(state: &InterceptorState) -> CacheLookupResult {
    // Find least-loaded pod from Redis pod registry
    match state.redis.list_pod_loads().await {
        Ok(pods) if !pods.is_empty() => {
            let (pod_id, _load) = &pods[0];
            CacheLookupResult::Miss {
                pod_ip: state.cfg.default_pod_ip.clone(),
                pod_id: pod_id.clone(),
            }
        }
        _ => CacheLookupResult::Miss {
            pod_ip: state.cfg.default_pod_ip.clone(),
            pod_id: "default".to_string(),
        },
    }
}

fn pod_url(pod_ip: &str, cfg: &SidecarConfig) -> String {
    format!("http://{}:{}{}", pod_ip, cfg.pod_port, cfg.pod_path)
}

/// Heuristic TTFT savings estimate in milliseconds.
/// Based on ~0.04ms per cached token (empirical p50 from vLLM benchmarks).
fn estimate_ttft_savings(cached_tokens: u32) -> i64 {
    (cached_tokens as f64 * 0.04).round() as i64
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn unix_now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}
