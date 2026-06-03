//! RetrievalRun aggregate root (PRD §11.5, retrieval.md). Immutable once built.

use crate::retrieval::domain::explain::ExplainPayload;
use crate::retrieval::domain::query::{Query, SearchMode};
use crate::retrieval::domain::retrieval_result::RetrievalResult;

/// Why a run produced the results it did — distinguishes "empty index" (AC-8)
/// from "ran fine, no matches" (AC-5/E4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunReason {
    Ok,
    IndexEmpty,
}

/// The authoritative record of one search execution. Never modified after creation.
#[derive(Debug, Clone)]
pub struct RetrievalRun {
    pub id: String,
    pub query: Query,
    /// Ordered by rank ascending (rank 1 = best). `len() <= top_k`.
    pub results: Vec<RetrievalResult>,
    pub search_mode: SearchMode,
    pub top_k: usize,
    pub latency_ms: u128,
    /// Results with score < SIMILARITY_THRESHOLD (E10; feeds M3/M4).
    pub below_threshold_count: usize,
    pub reason: RunReason,
    pub explain: Option<ExplainPayload>,
    pub completed_at: String,
}
