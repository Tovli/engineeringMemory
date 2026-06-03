//! ExplainPayload — the observability artefact for `--explain` (PRD §7, FR-SRCH-004, AC-6).

use crate::ingestion::domain::ChunkId;
use crate::retrieval::domain::query::MetadataFilter;

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainPayload {
    pub query_embedding_provider: String,
    pub query_embedding_dimension: usize,
    pub search_mode: String,
    pub filters_applied: MetadataFilter,
    /// e.g. "cosine"; in M5 hybrid this becomes "rrf".
    pub ranking_method: String,
    pub result_details: Vec<ExplainResultDetail>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainResultDetail {
    pub chunk_id: ChunkId,
    pub rank: usize,
    pub vector_score: Option<f32>,
    pub keyword_score: Option<f32>,
    pub fused_score: f32,
    /// Why this chunk was eligible — knn position + which filters it passed.
    pub eligibility_reason: String,
}
