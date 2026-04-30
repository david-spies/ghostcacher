// ghostcacher-control-plane/src/eviction.rs
// Cost-Weighted TTL Eviction Engine
//
// Standard LRU is insufficient for LLM KV caches because not all blocks
// have equal value. This engine implements a scoring function that weighs:
//   - Block type priority (System > Tools > Document > Session)
//   - Access frequency (hit_count)
//   - Recency (last_hit_at)
//   - Token density (cost per re-prefill if evicted)
//
// Eviction score (lower = evict first):
//   score = (type_priority × hit_count × token_count) / age_hours
//
// The engine runs on a configurable schedule and coordinates with Redis
// to trim entries that exceed the cluster's memory budget.

use anyhow::Result;
use std::{sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use tracing::{debug, info, warn};

use crate::config::ControlPlaneConfig;
use crate::pod_registry::PodRegistry;

const EVICTION_INTERVAL_SECS: u64 = 300; // Run every 5 minutes
const MAX_CACHE_ENTRIES: usize = 10_000;

pub struct EvictionEngine {
    pods: Arc<PodRegistry>,
    cfg:  Arc<ControlPlaneConfig>,
}

#[derive(Debug)]
struct EvictionCandidate {
    key:   String,
    score: f64,
}

impl EvictionEngine {
    pub fn new(pods: Arc<PodRegistry>, cfg: Arc<ControlPlaneConfig>) -> Self {
        Self { pods, cfg }
    }

    /// Background eviction loop — runs continuously
    pub async fn run_loop(&self) {
        let interval = Duration::from_secs(EVICTION_INTERVAL_SECS);
        info!("Eviction engine started (interval: {}s)", EVICTION_INTERVAL_SECS);
        loop {
            tokio::time::sleep(interval).await;
            if let Err(e) = self.run_eviction_cycle().await {
                warn!(%e, "Eviction cycle error");
            }
        }
    }

    async fn run_eviction_cycle(&self) -> Result<()> {
        info!("Starting eviction cycle");

        // In production: connect directly to Redis, SCAN all cache_map:* keys,
        // deserialize each CacheEntry, compute eviction score, sort, and
        // DEL the bottom N% that exceed the memory budget.
        //
        // This skeleton demonstrates the scoring algorithm and control flow.

        let now = unix_now_f64();
        let mut candidates: Vec<EvictionCandidate> = Vec::new();

        // Simulated candidate scan (production: SCAN cache_map:*)
        // Each entry is scored and candidates below threshold are pruned
        debug!("Scanning Redis for eviction candidates");

        // Score function for a cache entry
        // In production, deserialize CacheEntry from Redis JSON
        let _score_entry = |type_prio: f64, hit_count: f64, tokens: f64, last_hit: f64| -> f64 {
            let age_hours = ((now - last_hit) / 3600.0).max(0.01);
            (type_prio * hit_count.max(1.0) * tokens) / age_hours
        };

        // Entries with score below this threshold are eviction candidates
        // In production: compute dynamically based on total entry count
        let eviction_threshold = 0.5_f64;

        // Sort ascending — lowest score = evict first
        candidates.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

        let evict_count = candidates.len().saturating_sub(MAX_CACHE_ENTRIES);
        if evict_count == 0 {
            debug!("No eviction needed — cache within budget");
            return Ok(());
        }

        info!(
            candidates = candidates.len(),
            evicting = evict_count,
            "Eviction cycle complete"
        );

        Ok(())
    }

    /// Force-flush all cache entries matching a scope.
    /// Called by the admin API (/gc/cache/flush).
    pub async fn flush(&self, scope: &str) -> Result<u64> {
        info!(%scope, "Manual cache flush triggered");
        // Production: SCAN + DEL matching cache_map:* keys in Redis
        // Scope: "system" (only infinite-TTL), "docs", "session", "all"
        match scope {
            "all"     => info!("Flushing ALL cache entries"),
            "system"  => info!("Flushing system prompt entries only"),
            "docs"    => info!("Flushing RAG document entries"),
            "session" => info!("Flushing session entries"),
            _         => warn!(%scope, "Unknown flush scope"),
        }
        Ok(0) // Return count of flushed entries
    }
}

fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
