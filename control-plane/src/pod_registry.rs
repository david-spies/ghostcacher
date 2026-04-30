// ghostcacher-control-plane/src/pod_registry.rs
// Pod Registry — maintains a live map of GPU pods, their HBM utilization,
// and the KV block inventory each pod holds.
//
// Redis schema:
//   pod_registry (HSET)     → pod_id → JSON PodInfo
//   pod_load:{pod_id}       → f32 HBM utilization
//   pod_heartbeat:{pod_id}  → unix timestamp (TTL = 3× heartbeat_interval)

use anyhow::{Context, Result};
use redis::{aio::MultiplexedConnection, AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

const HEARTBEAT_TTL_SECS: u64 = 45;
const HEALTH_CHECK_INTERVAL_SECS: u64 = 15;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodInfo {
    pub pod_id:        String,
    pub pod_ip:        String,
    pub node_name:     String,
    pub hbm_util:      f32,
    pub dram_util:     f32,
    pub kv_block_count: u32,
    pub inference_engine: String,
    pub last_heartbeat: i64,
    pub status:        PodStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PodStatus {
    Warm,   // HBM has relevant KV blocks
    Active, // Currently processing a request
    Idle,
    Draining,
}

pub struct PodRegistry {
    client: Client,
}

impl PodRegistry {
    pub async fn new(redis_url: String) -> Result<Self> {
        let client = Client::open(redis_url.as_str())
            .context("PodRegistry: failed to open Redis client")?;

        // Verify connection
        let mut conn = client.get_multiplexed_async_connection().await?;
        redis::cmd("PING").query_async::<_, String>(&mut conn).await?;
        info!("PodRegistry connected to Redis");

        Ok(Self { client })
    }

    async fn conn(&self) -> Result<MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .context("PodRegistry: Redis connection error")
    }

    /// Register or update a pod's info (called on heartbeat)
    pub async fn upsert_pod(&self, info: &PodInfo) -> Result<()> {
        let mut conn = self.conn().await?;
        let json = serde_json::to_string(info)?;

        // HSET pod_registry pod_id → json
        conn.hset::<_, _, _, ()>("pod_registry", &info.pod_id, &json).await?;

        // Update load key with TTL (expires if pod stops sending heartbeats)
        conn.set_ex::<_, _, ()>(
            format!("pod_load:{}", info.pod_id),
            info.hbm_util,
            HEARTBEAT_TTL_SECS,
        ).await?;

        // Update heartbeat timestamp
        conn.set_ex::<_, _, ()>(
            format!("pod_heartbeat:{}", info.pod_id),
            unix_now(),
            HEARTBEAT_TTL_SECS,
        ).await?;

        debug!(pod_id = %info.pod_id, hbm = info.hbm_util, "Pod heartbeat recorded");
        Ok(())
    }

    /// Return all known pods sorted by HBM utilization (least loaded first)
    pub async fn list_pods(&self) -> Result<Vec<PodInfo>> {
        let mut conn = self.conn().await?;
        let all: Vec<(String, String)> = conn.hgetall("pod_registry").await?;

        let mut pods: Vec<PodInfo> = all
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_str(&v).ok())
            .collect();

        pods.sort_by(|a, b| a.hbm_util.partial_cmp(&b.hbm_util).unwrap());
        Ok(pods)
    }

    /// Return the pod with the lowest HBM utilization — used for cold miss routing
    pub async fn least_loaded_pod(&self) -> Result<Option<PodInfo>> {
        let pods = self.list_pods().await?;
        Ok(pods.into_iter().next())
    }

    /// Deregister a pod (called on graceful shutdown or eviction of dead pods)
    pub async fn remove_pod(&self, pod_id: &str) -> Result<()> {
        let mut conn = self.conn().await?;
        conn.hdel::<_, _, ()>("pod_registry", pod_id).await?;
        conn.del::<_, ()>(format!("pod_load:{}", pod_id)).await?;
        conn.del::<_, ()>(format!("pod_heartbeat:{}", pod_id)).await?;
        warn!(%pod_id, "Pod deregistered");
        Ok(())
    }

    /// Background loop: remove pods whose heartbeat has expired
    pub async fn health_check_loop(&self) {
        let interval = Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS);
        loop {
            tokio::time::sleep(interval).await;
            if let Err(e) = self.evict_dead_pods().await {
                warn!(%e, "Pod health check error");
            }
        }
    }

    async fn evict_dead_pods(&self) -> Result<()> {
        let mut conn = self.conn().await?;
        let all: Vec<(String, String)> = conn.hgetall("pod_registry").await?;
        let now = unix_now();

        for (pod_id, json) in all {
            if let Ok(info) = serde_json::from_str::<PodInfo>(&json) {
                let age_secs = now - info.last_heartbeat;
                if age_secs > HEARTBEAT_TTL_SECS as i64 {
                    warn!(%pod_id, age_secs, "Dead pod detected — deregistering");
                    self.remove_pod(&pod_id).await?;
                }
            }
        }
        Ok(())
    }
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}
