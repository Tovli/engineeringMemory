//! RetrievedContext + ContextChunk value objects (answer-generation.md). The bridge between
//! Retrieval's output model and the LLM prompt. Constructed inside this context only.

use crate::ingestion::domain::ChunkId;

/// One chunk selected to ground the answer. `text` is the retrieved chunk text the model sees.
/// In M4 this is the RetrievalResult `preview` (the only chunk text the index carries — see the
/// M4 completion doc); a future refinement may serve full chunk content.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextChunk {
    pub rank: usize,
    pub chunk_id: ChunkId,
    pub source_path: String,
    pub heading_path: Vec<String>,
    pub text: String,
    pub score: f32,
}

/// The assembled, token-budgeted context handed to the prompt renderer.
#[derive(Debug, Clone)]
pub struct RetrievedContext {
    pub query_text: String,
    /// Eligible chunks (score ≥ threshold), rank-ordered, trimmed to the token budget.
    pub chunks: Vec<ContextChunk>,
}
