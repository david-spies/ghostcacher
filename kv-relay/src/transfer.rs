// ghostcacher-kv-relay/src/transfer.rs
use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// A single KV attention block (one PagedAttention page)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvBlock {
    /// Which layer this block belongs to (0..num_layers)
    pub layer_idx: u32,
    /// Block index within the layer's KV cache
    pub block_idx: u32,
    /// Raw serialized K and V tensors (bf16 / fp16 packed)
    pub k_data: Vec<u8>,
    pub v_data: Vec<u8>,
    /// Number of tokens this block covers
    pub token_count: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    pub prefix_hash:   String,
    pub source_pod_ip: String,
    pub source_pod_id: String,
    /// Layer range to transfer (None = all layers)
    pub layer_range:   Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResponse {
    pub prefix_hash:       String,
    pub status:            TransferStatus,
    pub blocks_received:   u32,
    pub bytes_transferred: u64,
    pub transfer_ms:       u64,
    pub transport:         String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Success,
    PartialSuccess { blocks_failed: u32 },
    Failed { reason: String },
}
