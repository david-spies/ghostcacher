// ghostcacher-sidecar/src/config.rs
// Configuration loader — env vars take precedence over config file, then defaults.
// Compatible with Kubernetes ConfigMap / Secret injection.

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SidecarConfig {
    /// Address the sidecar proxy listens on
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Prometheus metrics endpoint address
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Upstream LLM provider URL (Anthropic / OpenAI / pod IP)
    #[serde(default = "default_upstream_url")]
    pub upstream_url: String,

    /// Redis cluster URL (redis://... or rediss://... for TLS)
    #[serde(default = "default_redis_url")]
    pub redis_url: String,

    /// Fallback pod IP for cold-miss routing when Redis is unavailable
    #[serde(default = "default_pod_ip")]
    pub default_pod_ip: String,

    /// Port on the GPU pod that the inference server listens on
    #[serde(default = "default_pod_port")]
    pub pod_port: u16,

    /// Path on the GPU pod (e.g. "/v1/messages")
    #[serde(default = "default_pod_path")]
    pub pod_path: String,

    /// Max request timeout in seconds (proxy to upstream)
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,

    /// HBM utilization threshold (0.0..1.0) above which CPU swap is triggered
    #[serde(default = "default_hbm_threshold")]
    pub hbm_swap_threshold: f32,

    /// Ghost-Lock acquisition timeout in seconds
    #[serde(default = "default_ghost_lock_timeout")]
    pub ghost_lock_timeout_secs: u64,

    /// Enable RDMA cross-node KV tensor transfer
    #[serde(default = "default_true")]
    pub rdma_enabled: bool,

    /// Log level override (e.g. "debug", "info", "warn")
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl SidecarConfig {
    /// Load configuration from environment variables.
    /// Each field maps to a GC_<FIELD_NAME_UPPERCASE> env var.
    pub fn load() -> Result<Self> {
        let cfg = config::Config::builder()
            .add_source(
                config::Environment::with_prefix("GC")
                    .separator("_")
                    .try_parsing(true),
            )
            .build()?;

        Ok(cfg.try_deserialize().unwrap_or_else(|_| SidecarConfig::default()))
    }
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            listen_addr:           default_listen_addr(),
            metrics_addr:          default_metrics_addr(),
            upstream_url:          default_upstream_url(),
            redis_url:             default_redis_url(),
            default_pod_ip:        default_pod_ip(),
            pod_port:              default_pod_port(),
            pod_path:              default_pod_path(),
            request_timeout_secs:  default_timeout(),
            hbm_swap_threshold:    default_hbm_threshold(),
            ghost_lock_timeout_secs: default_ghost_lock_timeout(),
            rdma_enabled:          true,
            log_level:             default_log_level(),
        }
    }
}

fn default_listen_addr()        -> String  { "0.0.0.0:8080".to_string() }
fn default_metrics_addr()       -> String  { "0.0.0.0:9090".to_string() }
fn default_upstream_url()       -> String  { "https://api.anthropic.com/v1/messages".to_string() }
fn default_redis_url()          -> String  { "redis://redis:6379".to_string() }
fn default_pod_ip()             -> String  { "127.0.0.1".to_string() }
fn default_pod_port()           -> u16     { 8000 }
fn default_pod_path()           -> String  { "/v1/messages".to_string() }
fn default_timeout()            -> u64     { 120 }
fn default_hbm_threshold()      -> f32     { 0.85 }
fn default_ghost_lock_timeout() -> u64     { 30 }
fn default_log_level()          -> String  { "info".to_string() }
fn default_true()               -> bool    { true }
