// ghostcacher-kv-relay/src/config.rs
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RelayConfig {
    #[serde(default = "default_grpc")]
    pub grpc_addr: String,

    #[serde(default = "default_http")]
    pub http_addr: String,

    #[serde(default = "default_node_ip")]
    pub node_ip: String,

    #[serde(default)]
    pub rdma_available: bool,

    /// Max concurrent KV transfer streams
    #[serde(default = "default_max_streams")]
    pub max_concurrent_streams: usize,

    /// Chunk size for gRPC streaming (bytes)
    #[serde(default = "default_chunk_size")]
    pub chunk_size_bytes: usize,
}

impl RelayConfig {
    pub fn load() -> Result<Self> {
        let cfg = config::Config::builder()
            .add_source(config::Environment::with_prefix("GC_RELAY").separator("_").try_parsing(true))
            .build()?;
        Ok(cfg.try_deserialize().unwrap_or_default())
    }
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            grpc_addr:              default_grpc(),
            http_addr:              default_http(),
            node_ip:                default_node_ip(),
            rdma_available:         false,
            max_concurrent_streams: default_max_streams(),
            chunk_size_bytes:       default_chunk_size(),
        }
    }
}

fn default_grpc()        -> String { "0.0.0.0:50051".to_string() }
fn default_http()        -> String { "0.0.0.0:50052".to_string() }
fn default_node_ip()     -> String { "127.0.0.1".to_string() }
fn default_max_streams() -> usize  { 16 }
fn default_chunk_size()  -> usize  { 4 * 1024 * 1024 } // 4 MB
