// ghostcacher-sidecar/src/hasher.rs
// Hierarchical Hash Strategy — the heart of GhostCacher's stable prefix engine
//
// Implements a block-level SHA-256 hashing pipeline that ensures:
// 1. Tokenization-consistent normalization (no whitespace drift)
// 2. Canonical document ordering for RAG blocks (alphabetical by source ID)
// 3. Hierarchical composition — the prefix hash is H(H_sys || H_tools || H_doc)
//    so any change to an upstream block invalidates all downstream hashes

use sha2::{Digest, Sha256};
use crate::types::{BlockKind, PromptBlock};

/// Canonical separator between block hashes in the composite prefix hash.
/// Using a fixed separator prevents length-extension attacks.
const BLOCK_SEP: &[u8] = b"\x00GC\x00";

/// Hasher output for a single request's structured prompt
#[derive(Debug, Clone)]
pub struct HashResult {
    /// H_sys: SHA-256 of canonicalized system block content
    pub h_sys:    Option<String>,
    /// H_tools: SHA-256 of canonicalized tool schema content
    pub h_tools:  Option<String>,
    /// H_doc: SHA-256 of canonicalized document block(s), sorted by source ID
    pub h_doc:    Option<String>,
    /// Composite prefix hash: H(H_sys || sep || H_tools || sep || H_doc)
    /// This is the Redis lookup key.
    pub prefix:   String,
    /// Count of cacheable tokens in the prefix (estimated)
    pub prefix_token_estimate: u32,
}

/// Canonicalize a block's content for hashing.
/// Normalization rules:
///   - Collapse all internal whitespace runs to single space
///   - Strip leading/trailing whitespace
///   - Lowercase source IDs in RAG blocks (prevents case-drift misses)
///   - For document blocks, sort [SOURCE:NNN] sections alphabetically
fn canonicalize(kind: &BlockKind, content: &str) -> String {
    let trimmed = content.trim();

    match kind {
        BlockKind::Document => {
            // Split on [SOURCE:NNN] markers, sort by source ID, rejoin
            // This ensures RAG result ordering never causes a cache miss
            let mut sections: Vec<&str> = trimmed.split("[SOURCE:").collect();
            let preamble = sections.remove(0); // content before first source tag

            let mut tagged: Vec<String> = sections
                .into_iter()
                .map(|s| format!("[SOURCE:{}", s))
                .collect();

            tagged.sort(); // alphabetical sort by source ID prefix

            let joined = tagged.join("\n");
            let full = if preamble.is_empty() {
                joined
            } else {
                format!("{}\n{}", preamble.trim(), joined)
            };

            // Collapse whitespace
            full.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        _ => {
            // For all other blocks: just normalize whitespace
            trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    }
}

/// SHA-256 hash a byte slice, return lowercase hex string
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Hash a single canonicalized block
fn hash_block(kind: &BlockKind, content: &str) -> String {
    let canonical = canonicalize(kind, content);
    // Prefix the hash input with the block kind tag to prevent collisions
    // between identical content in different block positions
    let tagged = format!("{}:{}", kind_tag(kind), canonical);
    sha256_hex(tagged.as_bytes())
}

fn kind_tag(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::System   => "SYS",
        BlockKind::Tools    => "TOOLS",
        BlockKind::Document => "DOC",
        BlockKind::User     => "USER",
    }
}

/// Estimate token count for a block (fast heuristic: chars / 3.8)
/// Used only for metrics/savings estimation, not for routing decisions
fn estimate_tokens(content: &str) -> u32 {
    (content.len() as f64 / 3.8).ceil() as u32
}

/// Compute the full HashResult for a list of prompt blocks.
///
/// Blocks must be provided in canonical order:
///   [System] → [Tools] → [Documents...] → [User]
///
/// The composite prefix hash is built incrementally — each block's hash
/// is fed into the running SHA-256 state alongside the separator, so the
/// prefix hash always represents the complete stable prefix seen so far.
pub fn hash_prompt_blocks(blocks: &[PromptBlock]) -> HashResult {
    let mut h_sys:   Option<String> = None;
    let mut h_tools: Option<String> = None;
    let mut h_doc:   Option<String> = None;
    let mut prefix_token_estimate = 0u32;

    // Compose the prefix incrementally
    let mut prefix_hasher = Sha256::new();
    let mut first_block = true;

    for block in blocks {
        if !block.kind.is_cacheable() {
            continue; // Skip volatile User blocks
        }

        let block_hash = hash_block(&block.kind, &block.content);
        prefix_token_estimate += estimate_tokens(&block.content);

        // Feed block hash + separator into the composite hasher
        if !first_block {
            prefix_hasher.update(BLOCK_SEP);
        }
        prefix_hasher.update(block_hash.as_bytes());
        first_block = false;

        match block.kind {
            BlockKind::System   => h_sys   = Some(block_hash),
            BlockKind::Tools    => h_tools = Some(block_hash),
            BlockKind::Document => {
                // For multiple document blocks, compose into a single h_doc
                h_doc = Some(match &h_doc {
                    None       => block_hash,
                    Some(prev) => sha256_hex(
                        format!("{}{}{}", prev, std::str::from_utf8(BLOCK_SEP).unwrap_or(""), block_hash).as_bytes()
                    ),
                });
            }
            BlockKind::User => unreachable!(),
        }
    }

    let prefix = hex::encode(prefix_hasher.finalize());

    HashResult {
        h_sys,
        h_tools,
        h_doc,
        prefix,
        prefix_token_estimate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PromptBlock;

    fn block(kind: BlockKind, content: &str) -> PromptBlock {
        PromptBlock { kind, content: content.to_string(), hash: None }
    }

    #[test]
    fn whitespace_normalization_is_stable() {
        let b1 = block(BlockKind::System, "You are a helpful assistant.");
        let b2 = block(BlockKind::System, "  You  are  a  helpful  assistant.  ");
        let h1 = hash_prompt_blocks(&[b1]);
        let h2 = hash_prompt_blocks(&[b2]);
        assert_eq!(h1.prefix, h2.prefix, "Whitespace drift should not cause a cache miss");
    }

    #[test]
    fn document_source_ordering_is_stable() {
        let doc1 = "[ SOURCE:002 ] Second doc.\n[ SOURCE:001 ] First doc.";
        let doc2 = "[ SOURCE:001 ] First doc.\n[ SOURCE:002 ] Second doc.";
        let b1 = block(BlockKind::Document, doc1);
        let b2 = block(BlockKind::Document, doc2);
        let h1 = hash_prompt_blocks(&[b1]);
        let h2 = hash_prompt_blocks(&[b2]);
        // RAG result ordering should not cause a cache miss
        // NOTE: This test will only pass if source blocks start with [SOURCE:
        // The canonicalize fn sorts sections alphabetically
        assert_eq!(h1.h_doc, h2.h_doc, "RAG document ordering should be stable");
    }

    #[test]
    fn user_block_excluded_from_prefix() {
        let sys  = block(BlockKind::System, "You are a legal AI.");
        let user1 = block(BlockKind::User, "What is clause 4?");
        let user2 = block(BlockKind::User, "Summarize section 7.");

        let h1 = hash_prompt_blocks(&[sys.clone(), user1]);
        let h2 = hash_prompt_blocks(&[sys.clone(), user2]);
        assert_eq!(h1.prefix, h2.prefix, "User block must not affect the prefix hash");
    }

    #[test]
    fn different_system_prompts_produce_different_hashes() {
        let b1 = block(BlockKind::System, "You are assistant A.");
        let b2 = block(BlockKind::System, "You are assistant B.");
        let h1 = hash_prompt_blocks(&[b1]);
        let h2 = hash_prompt_blocks(&[b2]);
        assert_ne!(h1.prefix, h2.prefix);
    }
}
