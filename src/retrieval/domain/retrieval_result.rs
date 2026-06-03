//! RetrievalResult value object (PRD §11.5, retrieval.md).

use std::collections::BTreeMap;

use crate::ingestion::domain::{ChunkId, DocumentId};

/// One scored, ranked hit. `score` is a similarity in [0,1], higher = better
/// (normalized from cosine distance — ADR-0003).
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalResult {
    /// 1-based; rank 1 is the best match. Unique within a RetrievalRun.
    pub rank: usize,
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub source_path: String,
    pub score: f32,
    pub preview: String,
    pub heading_path: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}
