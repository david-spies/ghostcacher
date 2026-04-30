// ghostcacher-control-plane/src/config.rs
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ControlPlaneConfig {
    #[serde(default = "default_listen")]
    pub listen_addr: String,

    #[serde(default = "default_redis")]
    pub redis_url: String,

    #[serde(default = "default_eviction_interval")]
    pub eviction_interval_secs: u64,

    #[serde(default = "default_max_entries")]
    pub max_cache_entries: usize,

    #[serde(default = "default_hbm_swap")]
    pub hbm_swap_threshold: f32,
}

impl ControlPlaneConfig {
    pub fn load() -> Result<Self> {
        let cfg = config::Config::builder()
            .add_source(config::Environment::with_prefix("GC_CP").separator("_").try_parsing(true))
            .build()?;
        Ok(cfg.try_deserialize().unwrap_or_default())
    }
}

impl Default for ControlPlaneConfig {
    fn default() -> Self {
        Self {
            listen_addr:            default_listen(),
            redis_url:              default_redis(),
            eviction_interval_secs: default_eviction_interval(),
            max_cache_entries:      default_max_entries(),
            hbm_swap_threshold:     default_hbm_swap(),
        }
    }
}

fn default_listen()             -> String  { "0.0.0.0:7070".to_string() }
fn default_redis()              -> String  { "redis://redis:6379".to_string() }
fn default_eviction_interval()  -> u64     { 300 }
fn default_max_entries()        -> usize   { 10_000 }
fn default_hbm_swap()           -> f32     { 0.85 }
