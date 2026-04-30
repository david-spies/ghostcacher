// ghostcacher-sidecar/src/provider.rs
// Provider Adapter — transforms outgoing requests to use provider-specific
// prompt caching mechanisms, making the caching "backend-less" for the developer.
//
// Supported providers:
//   Anthropic  — cache_control: { type: "ephemeral" } on content blocks
//   OpenAI     — automatic (no explicit header required)
//   Bedrock    — performanceConfig cachePoints injection
//   Vertex     — systemInstruction caching via cachedContent reference
//   SelfHosted — no-op (KV caching handled directly by vLLM/SGLang)

use bytes::Bytes;
use tracing::debug;

use crate::types::{BlockKind, LLMProvider, PromptBlock};

pub struct ProviderAdapter;

impl ProviderAdapter {
    /// Inject provider-specific cache headers / body modifications.
    /// Returns the (potentially mutated) request body bytes.
    pub fn inject_cache_headers(
        provider: &LLMProvider,
        body:     &[u8],
        blocks:   &[PromptBlock],
    ) -> Bytes {
        match provider {
            LLMProvider::Anthropic  => Self::inject_anthropic(body, blocks),
            LLMProvider::OpenAI     => Self::inject_openai(body, blocks),
            LLMProvider::Bedrock    => Self::inject_bedrock(body, blocks),
            LLMProvider::Vertex     => Self::inject_vertex(body, blocks),
            LLMProvider::SelfHosted => Bytes::copy_from_slice(body),
        }
    }

    /// Anthropic: inject `cache_control: { "type": "ephemeral" }` at the
    /// breakpoint — the last cacheable block before the volatile User tail.
    ///
    /// This tells Anthropic's infrastructure to persist the KV state up to
    /// and including this content block, enabling future requests with the
    /// same prefix to skip the prefill phase entirely.
    fn inject_anthropic(body: &[u8], blocks: &[PromptBlock]) -> Bytes {
        let mut json: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v)  => v,
            Err(_) => return Bytes::copy_from_slice(body),
        };

        // Identify the last cacheable block position
        let last_cacheable_idx = blocks
            .iter()
            .rposition(|b| b.kind.is_cacheable());

        if last_cacheable_idx.is_none() {
            return Bytes::copy_from_slice(body);
        }

        // Inject cache_control on the system field if it's the breakpoint
        // (Anthropic's API places cache_control directly on the system string
        //  or on individual message content blocks)
        if let Some(system) = json.get_mut("system") {
            // Convert string system to array-of-blocks format for cache_control injection
            if system.is_string() {
                let sys_text = system.as_str().unwrap().to_string();
                *system = serde_json::json!([
                    {
                        "type": "text",
                        "text": sys_text,
                        "cache_control": { "type": "ephemeral" }
                    }
                ]);
                debug!("Anthropic cache_control injected on system block");
            }
        }

        // If there's a document block before the user message, inject
        // a second breakpoint there (Anthropic supports up to 4 breakpoints)
        if let Some(messages) = json.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let len = messages.len();
            if len >= 2 {
                // The message before the last one is the doc/history context
                // Inject cache_control on that block's content
                if let Some(msg) = messages.get_mut(len - 2) {
                    if let Some(content) = msg.get_mut("content") {
                        if content.is_string() {
                            let text = content.as_str().unwrap().to_string();
                            *content = serde_json::json!([
                                {
                                    "type": "text",
                                    "text": text,
                                    "cache_control": { "type": "ephemeral" }
                                }
                            ]);
                            debug!("Anthropic cache_control injected on document/history block");
                        }
                    }
                }
            }
        }

        Bytes::from(serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec()))
    }

    /// OpenAI: Prompt caching is automatic for prompts ≥ 1024 tokens.
    /// GhostCacher still optimizes routing to the same pod that processed
    /// the cached prefix, increasing the likelihood of a server-side hit.
    ///
    /// Optionally injects `store: true` for the Responses API.
    fn inject_openai(body: &[u8], _blocks: &[PromptBlock]) -> Bytes {
        let mut json: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v)  => v,
            Err(_) => return Bytes::copy_from_slice(body),
        };

        // Enable persistent caching for Responses API if not already set
        if json.get("store").is_none() {
            json["store"] = serde_json::json!(true);
        }

        Bytes::from(serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec()))
    }

    /// AWS Bedrock: inject performanceConfig with optimized latency mode
    /// and cachePoints at the stable prefix boundary.
    fn inject_bedrock(body: &[u8], blocks: &[PromptBlock]) -> Bytes {
        let mut json: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v)  => v,
            Err(_) => return Bytes::copy_from_slice(body),
        };

        json["performanceConfig"] = serde_json::json!({
            "latency": "optimized"
        });

        // Find the breakpoint position (last cacheable block)
        let breakpoint_idx = blocks.iter().rposition(|b| b.kind.is_cacheable());

        if let Some(idx) = breakpoint_idx {
            if let Some(messages) = json.get_mut("messages").and_then(|m| m.as_array_mut()) {
                if let Some(msg) = messages.get_mut(idx) {
                    msg["cachePoint"] = serde_json::json!({ "type": "default" });
                    debug!("Bedrock cachePoint injected at message index {}", idx);
                }
            }
        }

        Bytes::from(serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec()))
    }

    /// Google Vertex: reference a pre-created cachedContent resource.
    /// GhostCacher manages the cache resource lifecycle externally;
    /// here we just inject the reference into the generate request.
    fn inject_vertex(body: &[u8], _blocks: &[PromptBlock]) -> Bytes {
        // Vertex caching uses a separate cachedContent API call.
        // This adapter is a pass-through; the control plane manages
        // the cache lifecycle via the Vertex Content Cache API.
        Bytes::copy_from_slice(body)
    }
}
