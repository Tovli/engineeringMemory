//! Domain types for the Ingestion context (pure — no engine/infra deps).
//! Mirrors docs/ddd/contexts/ingestion.md.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type DocumentId = String;
pub type ChunkId = String;
/// blake3 hex digest.
pub type ContentHash = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentStatus {
    Active,
    Modified,
    Deleted,
}

/// Which embedding model produced a chunk's vector (PRD FR-EMB-002, Risk 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingModelVersion {
    pub name: String,
    pub dimension: usize,
    pub created_at: String,
}

/// Chunk sizing config (PRD FR-CHK-002). Invariant: overlap < target < max.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ChunkingConfig {
    pub target_tokens: u32,
    pub max_tokens: u32,
    pub overlap_tokens: u32,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self { target_tokens: 500, max_tokens: 800, overlap_tokens: 80 }
    }
}

impl ChunkingConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.overlap_tokens < self.target_tokens && self.target_tokens < self.max_tokens {
            Ok(())
        } else {
            Err(format!(
                "invalid ChunkingConfig: require overlap < target < max, got {} < {} < {}",
                self.overlap_tokens, self.target_tokens, self.max_tokens
            ))
        }
    }
}

/// A chunk of a document (entity owned by IngestionDocument).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub id: ChunkId,
    pub document_id: DocumentId,
    pub chunk_index: u32,
    pub heading_path: Vec<String>,
    pub content: String,
    pub preview: String,
    pub content_hash: ContentHash,
    pub token_count: u32,
    pub metadata: BTreeMap<String, String>,
}

/// Aggregate root for one source file's lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionDocument {
    pub id: DocumentId,
    pub source_path: String,
    pub file_name: String,
    pub file_extension: String,
    pub content_hash: ContentHash,
    pub title: Option<String>,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub status: DocumentStatus,
    pub embedding_model: String,
    pub embedding_dimension: usize,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

/// Result of one `tovli ingest` execution (maps to the IngestionRun aggregate).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestionSummary {
    pub files_scanned: usize,
    pub files_ingested: usize,
    pub files_unchanged: usize,
    pub files_skipped: usize,
    pub files_errored: usize,
    pub files_empty: usize,
    pub files_deleted: usize,
    pub chunks_created: usize,
    /// (source_path, reason)
    pub errors: Vec<(String, String)>,
    /// (source_path, reason)
    pub skipped: Vec<(String, String)>,
    pub dry_run: bool,
}

/// blake3 hex digest of bytes — the canonical content hash (D-HASH).
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    blake3::hash(bytes).to_hex().to_string()
}

/// Deterministic, stable chunk id from document + index + content hash.
pub fn chunk_id(document_id: &str, chunk_index: u32, content_hash: &str) -> ChunkId {
    let h = blake3::hash(format!("{document_id}:{chunk_index}:{content_hash}").as_bytes());
    format!("chunk_{}", &h.to_hex()[..16])
}

/// Stable document id derived from source path.
pub fn document_id(source_path: &str) -> DocumentId {
    let h = blake3::hash(source_path.as_bytes());
    format!("doc_{}", &h.to_hex()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_config_invariant() {
        assert!(ChunkingConfig::default().validate().is_ok());
        assert!(ChunkingConfig { target_tokens: 500, max_tokens: 400, overlap_tokens: 80 }
            .validate()
            .is_err());
        assert!(ChunkingConfig { target_tokens: 100, max_tokens: 800, overlap_tokens: 200 }
            .validate()
            .is_err());
    }

    #[test]
    fn hashing_is_deterministic_and_stable() {
        assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
        assert_ne!(content_hash(b"hello"), content_hash(b"world"));
        let a = chunk_id("doc_1", 0, "abc");
        assert_eq!(a, chunk_id("doc_1", 0, "abc"));
        assert_ne!(a, chunk_id("doc_1", 1, "abc"));
        assert!(document_id("docs/a.md").starts_with("doc_"));
    }
}
