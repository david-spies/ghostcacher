// ghostcacher-sidecar/src/types.rs
// Shared domain types for GhostCacher

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single block in a structured GhostCacher prompt.
/// Blocks are processed in order: Sys → Tools → Docs → User.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    System,
    Tools,
    Document,
    User,
}

impl BlockKind {
    /// Returns true for blocks that participate in prefix caching.
    /// User blocks are always volatile — never cached.
    pub fn is_cacheable(&self) -> bool {
        !matches!(self, BlockKind::User)
    }

    /// Priority weight used for eviction scoring.
    /// Higher score = evict last.
    pub fn eviction_priority(&self) -> u32 {
        match self {
            BlockKind::System   => 1000,
            BlockKind::Tools    => 900,
            BlockKind::Document => 500,
            BlockKind::User     => 0,
        }
    }
}

/// Structured prompt block before hashing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptBlock {
    pub kind:    BlockKind,
    pub content: String,
    /// SHA-256 hex of the canonicalized content (set after hashing)
    pub hash:    Option<String>,
}

/// The result of a cache lookup for a prefix hash
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheLookupResult {
    /// Full cache hit — route to this specific pod
    Hit { pod_ip: String, pod_id: String, blocks_reused: u32 },
    /// Partial hit — system cached but doc is cold
    PartialHit { pod_ip: String, pod_id: String, blocks_reused: u32 },
    /// Complete miss — route to least-loaded pod, schedule write
    Miss { pod_ip: String, pod_id: String },
}

/// Routing decision emitted by the Affinity Router
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub request_id:    Uuid,
    pub lookup_result: CacheLookupResult,
    pub prefix_hash:   String,
    pub provider:      LLMProvider,
    /// Estimated prefill tokens saved (0 on miss)
    pub tokens_saved:  u32,
    /// Estimated TTFT delta in milliseconds
    pub ttft_delta_ms: i64,
}

/// Supported LLM providers with their cache injection strategies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LLMProvider {
    Anthropic,
    OpenAI,
    Bedrock,
    Vertex,
    /// Self-hosted via vLLM / SGLang (direct KV tensor management)
    SelfHosted,
}

impl LLMProvider {
    pub fn from_upstream_url(url: &str) -> Self {
        if url.contains("anthropic")    { return LLMProvider::Anthropic; }
        if url.contains("openai")       { return LLMProvider::OpenAI; }
        if url.contains("amazonaws")    { return LLMProvider::Bedrock; }
        if url.contains("googleapis")   { return LLMProvider::Vertex; }
        LLMProvider::SelfHosted
    }
}

/// Cache entry stored in Redis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub prefix_hash: String,
    pub pod_ip:      String,
    pub pod_id:      String,
    pub block_count: u32,
    pub token_count: u32,
    pub hit_count:   u64,
    pub created_at:  i64,
    pub last_hit_at: i64,
    pub ttl_policy:  TtlPolicy,
}

/// TTL policy variants for the cost-weighted eviction engine
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtlPolicy {
    /// System prompts / tool schemas — never expire automatically
    Infinite,
    /// RAG documents — expires N seconds after creation
    Fixed { secs: u64 },
    /// Session history — resets on every access
    Sliding { window_secs: u64 },
}

impl TtlPolicy {
    pub fn redis_ttl_secs(&self) -> Option<u64> {
        match self {
            TtlPolicy::Infinite              => None,
            TtlPolicy::Fixed { secs }        => Some(*secs),
            TtlPolicy::Sliding { window_secs } => Some(*window_secs),
        }
    }
}

/// Prometheus-compatible metric snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub cache_hit_ratio:   f64,
    pub total_hits:        u64,
    pub total_misses:      u64,
    pub saved_ttft_ms_avg: f64,
    pub tokens_cached:     u64,
    pub active_entries:    u64,
    pub rdma_transfers:    u64,
}
