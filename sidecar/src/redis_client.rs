// ghostcacher-sidecar/src/redis_client.rs
// Redis Control Plane — distributed hash-to-pod affinity map
//
// Schema:
//   cache_map:{prefix_hash}       → JSON CacheEntry (TTL per policy)
//   ghost_lock:{prefix_hash}      → "1" (NX, 30s TTL) — thundering herd prevention
//   pod_load:{pod_id}             → float 0..1 (HBM utilization, updated by pods)
//   pod_registry                  → HSET pod_id → JSON PodInfo

use anyhow::{Context, Result};
use redis::{
    aio::MultiplexedConnection, AsyncCommands, Client, RedisResult,
};
use serde_json;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

use crate::types::{CacheEntry, TtlPolicy};

const GHOST_LOCK_TTL_SECS: u64 = 30;
const GHOST_LOCK_RETRY_MAX: u32 = 8;
const GHOST_LOCK_BACKOFF_BASE_MS: u64 = 50;

pub struct RedisControlPlane {
    client: Client,
}

impl RedisControlPlane {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = Client::open(url)
            .context("Failed to open Redis client")?;

        // Verify connectivity
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to connect to Redis")?;

        redis::cmd("PING")
            .query_async::<_, String>(&mut conn)
            .await
            .context("Redis PING failed")?;

        info!(%url, "Redis control plane connection verified");
        Ok(Self { client })
    }

    async fn conn(&self) -> Result<MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .context("Redis connection error")
    }

    /// Look up the pod affinity for a prefix hash.
    /// Returns Some(CacheEntry) on hit, None on miss.
    pub async fn lookup(&self, prefix_hash: &str) -> Result<Option<CacheEntry>> {
        let mut conn = self.conn().await?;
        let key = format!("cache_map:{}", prefix_hash);

        let raw: Option<String> = conn.get(&key).await?;
        match raw {
            None => {
                debug!(%prefix_hash, "Cache MISS");
                Ok(None)
            }
            Some(json) => {
                let mut entry: CacheEntry = serde_json::from_str(&json)
                    .context("Failed to deserialize CacheEntry")?;

                // Refresh sliding TTL on hit
                if let TtlPolicy::Sliding { window_secs } = &entry.ttl_policy {
                    let secs = *window_secs;
                    conn.expire::<_, ()>(&key, secs as i64).await?;
                }

                // Update last_hit_at and hit_count
                let now = unix_now();
                entry.last_hit_at = now;
                entry.hit_count   += 1;
                let updated = serde_json::to_string(&entry)?;
                conn.set::<_, _, ()>(&key, &updated).await?;

                debug!(%prefix_hash, pod_ip = %entry.pod_ip, hit_count = entry.hit_count, "Cache HIT");
                Ok(Some(entry))
            }
        }
    }

    /// Write a new cache entry for a prefix hash.
    /// Respects the TtlPolicy — infinite entries have no EXPIRE set.
    pub async fn write(&self, entry: &CacheEntry) -> Result<()> {
        let mut conn = self.conn().await?;
        let key = format!("cache_map:{}", entry.prefix_hash);
        let json = serde_json::to_string(entry)?;

        match entry.ttl_policy.redis_ttl_secs() {
            None => {
                conn.set::<_, _, ()>(&key, &json).await?;
                debug!(hash = %entry.prefix_hash, "Cache entry written (infinite TTL)");
            }
            Some(secs) => {
                conn.set_ex::<_, _, ()>(&key, &json, secs).await?;
                debug!(hash = %entry.prefix_hash, ttl = secs, "Cache entry written");
            }
        }

        Ok(())
    }

    /// Acquire a Ghost-Lock for a prefix hash.
    ///
    /// Uses Redis SET NX PX to atomically acquire the lock.
    /// Only one pod across the cluster wins — all others retry with
    /// exponential backoff, then re-attempt the cache lookup.
    ///
    /// This prevents the "Thundering Herd" where N pods simultaneously
    /// prefill the same cold prompt block, wasting N-1 GPU-prefill cycles.
    ///
    /// Returns true if lock acquired, false if another pod holds it.
    pub async fn try_acquire_ghost_lock(&self, prefix_hash: &str) -> Result<bool> {
        let mut conn = self.conn().await?;
        let lock_key = format!("ghost_lock:{}", prefix_hash);

        let result: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(GHOST_LOCK_TTL_SECS)
            .query_async(&mut conn)
            .await?;

        let acquired = result.is_some();
        if acquired {
            debug!(%prefix_hash, "Ghost-Lock acquired");
        } else {
            debug!(%prefix_hash, "Ghost-Lock contended — another pod is warming this prefix");
        }
        Ok(acquired)
    }

    /// Release a Ghost-Lock (called after the warm write completes)
    pub async fn release_ghost_lock(&self, prefix_hash: &str) -> Result<()> {
        let mut conn = self.conn().await?;
        let lock_key = format!("ghost_lock:{}", prefix_hash);
        conn.del::<_, ()>(&lock_key).await?;
        debug!(%prefix_hash, "Ghost-Lock released");
        Ok(())
    }

    /// Wait for the Ghost-Lock on a prefix to be released (miss path, lock contended).
    /// Implements exponential backoff. After max retries, falls through to a fresh lookup.
    pub async fn await_ghost_lock_release(&self, prefix_hash: &str) -> Result<Option<CacheEntry>> {
        let mut attempt = 0u32;
        loop {
            if attempt >= GHOST_LOCK_RETRY_MAX {
                warn!(%prefix_hash, "Ghost-Lock wait exhausted — falling through to cold route");
                return Ok(None);
            }

            let backoff_ms = GHOST_LOCK_BACKOFF_BASE_MS * (1 << attempt.min(6));
            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
            attempt += 1;

            // Check if lock has been released
            let mut conn = self.conn().await?;
            let lock_key = format!("ghost_lock:{}", prefix_hash);
            let locked: bool = conn.exists(&lock_key).await?;

            if !locked {
                // Lock released — re-attempt lookup (should hit now)
                return self.lookup(prefix_hash).await;
            }
        }
    }

    /// Return the current HBM utilization (0.0..1.0) for a pod.
    /// Pods report this via the pod heartbeat endpoint.
    pub async fn get_pod_load(&self, pod_id: &str) -> Result<f32> {
        let mut conn = self.conn().await?;
        let key = format!("pod_load:{}", pod_id);
        let val: Option<f32> = conn.get(&key).await?;
        Ok(val.unwrap_or(0.5)) // Default to 50% if no heartbeat yet
    }

    /// Return all registered pod IDs with their load scores.
    /// Used by the Affinity Router to select the least-loaded pod on a miss.
    pub async fn list_pod_loads(&self) -> Result<Vec<(String, f32)>> {
        let mut conn = self.conn().await?;
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg("pod_load:*")
            .query_async(&mut conn)
            .await?;

        let mut loads = Vec::new();
        for key in &keys {
            let pod_id = key.strip_prefix("pod_load:").unwrap_or(key).to_string();
            let load: f32 = conn.get(key).await.unwrap_or(0.5);
            loads.push((pod_id, load));
        }

        // Sort by load ascending — least loaded first
        loads.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        Ok(loads)
    }

    /// Increment the hit counter for Prometheus export
    pub async fn incr_metric(&self, key: &str) -> Result<()> {
        let mut conn = self.conn().await?;
        conn.incr::<_, _, ()>(format!("gc_metric:{}", key), 1).await?;
        Ok(())
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}
