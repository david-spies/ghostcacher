// ghostcacher-sidecar/src/metrics.rs
// Prometheus + OpenTelemetry metrics for GhostCacher
//
// Exported metrics:
//   gc_cache_hits_total           — counter
//   gc_cache_misses_total         — counter
//   gc_cache_hit_ratio            — gauge (rolling)
//   gc_tokens_cached_total        — counter
//   gc_saved_ttft_ms_total        — counter (sum of all saved TTFT ms)
//   gc_rdma_transfers_total       — counter
//   gc_active_cache_entries       — gauge
//   gc_request_latency_ms         — histogram (sidecar overhead only)

use anyhow::Result;
use prometheus::{
    Counter, Gauge, Histogram, HistogramOpts, IntCounter, Opts, Registry, TextEncoder,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct MetricsRegistry {
    pub hits:           IntCounter,
    pub misses:         IntCounter,
    pub tokens_cached:  IntCounter,
    pub ttft_saved_ms:  IntCounter,
    pub rdma_transfers: IntCounter,
    pub hit_ratio:      Gauge,
    pub active_entries: Gauge,
    pub latency_ms:     Histogram,
    registry:           Registry,
    // Internal counters for ratio computation
    total_hits:   AtomicU64,
    total_misses: AtomicU64,
}

impl MetricsRegistry {
    pub fn new() -> Result<Self> {
        let registry = Registry::new();

        let hits = IntCounter::with_opts(Opts::new(
            "gc_cache_hits_total",
            "Total number of GhostCacher cache hits",
        ))?;

        let misses = IntCounter::with_opts(Opts::new(
            "gc_cache_misses_total",
            "Total number of GhostCacher cache misses",
        ))?;

        let tokens_cached = IntCounter::with_opts(Opts::new(
            "gc_tokens_cached_total",
            "Cumulative count of prompt tokens served from cache",
        ))?;

        let ttft_saved_ms = IntCounter::with_opts(Opts::new(
            "gc_saved_ttft_ms_total",
            "Cumulative TTFT milliseconds saved by cache hits",
        ))?;

        let rdma_transfers = IntCounter::with_opts(Opts::new(
            "gc_rdma_transfers_total",
            "Total KV tensor transfers via RDMA between GPU nodes",
        ))?;

        let hit_ratio = Gauge::with_opts(Opts::new(
            "gc_cache_hit_ratio",
            "Rolling cache hit ratio (0.0 to 1.0)",
        ))?;

        let active_entries = Gauge::with_opts(Opts::new(
            "gc_active_cache_entries",
            "Current number of active cache entries in Redis",
        ))?;

        let latency_ms = Histogram::with_opts(HistogramOpts::new(
            "gc_request_latency_ms",
            "GhostCacher sidecar overhead latency in milliseconds",
        ).buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0]))?;

        registry.register(Box::new(hits.clone()))?;
        registry.register(Box::new(misses.clone()))?;
        registry.register(Box::new(tokens_cached.clone()))?;
        registry.register(Box::new(ttft_saved_ms.clone()))?;
        registry.register(Box::new(rdma_transfers.clone()))?;
        registry.register(Box::new(hit_ratio.clone()))?;
        registry.register(Box::new(active_entries.clone()))?;
        registry.register(Box::new(latency_ms.clone()))?;

        Ok(Self {
            hits,
            misses,
            tokens_cached,
            ttft_saved_ms,
            rdma_transfers,
            hit_ratio,
            active_entries,
            latency_ms,
            registry,
            total_hits:   AtomicU64::new(0),
            total_misses: AtomicU64::new(0),
        })
    }

    pub fn record_hit(&self, tokens_saved: u32) {
        self.hits.inc();
        self.tokens_cached.inc_by(tokens_saved as u64);

        // Estimate TTFT savings: ~0.04ms per token at p50
        let ttft_ms = (tokens_saved as f64 * 0.04).round() as u64;
        self.ttft_saved_ms.inc_by(ttft_ms);

        let h = self.total_hits.fetch_add(1, Ordering::Relaxed) + 1;
        let m = self.total_misses.load(Ordering::Relaxed);
        self.update_hit_ratio(h, m);
    }

    pub fn record_miss(&self) {
        self.misses.inc();
        let h = self.total_hits.load(Ordering::Relaxed);
        let m = self.total_misses.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_hit_ratio(h, m);
    }

    pub fn record_rdma_transfer(&self) {
        self.rdma_transfers.inc();
    }

    pub fn record_latency(&self, ms: f64) {
        self.latency_ms.observe(ms);
    }

    pub fn set_active_entries(&self, count: f64) {
        self.active_entries.set(count);
    }

    fn update_hit_ratio(&self, hits: u64, misses: u64) {
        let total = hits + misses;
        if total > 0 {
            self.hit_ratio.set(hits as f64 / total as f64);
        }
    }

    /// Render Prometheus text format for the /metrics endpoint
    pub fn render(&self) -> String {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        encoder.encode_to_string(&families).unwrap_or_default()
    }
}
