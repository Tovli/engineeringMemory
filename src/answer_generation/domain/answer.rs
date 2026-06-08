//! Answer aggregate root + Citation / NoAnswerReason value objects (answer-generation.md).
//! Serialized (camelCase) into the answer log (FR-RAG-004 / R9). Immutable once built.

use serde::Serialize;

use crate::ingestion::domain::ChunkId;

/// Why the system declined to produce a grounded answer (FR-RAG-003). Serializes to the exact
/// camelCase strings the DDD model uses (e.g. `belowSimilarityThreshold`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoAnswerReason {
    /// No retrieved result cleared `SIMILARITY_THRESHOLD` — refuse before calling the LLM (AC-2).
    BelowSimilarityThreshold,
    /// The LLM produced no usable, source-grounded answer (no/invalid citations or empty text).
    OutsideCorpus,
    /// Retrieved chunks contradict each other (best-effort; see ADR-0008, deferred in M4).
    SourcesConflict,
    /// The LLM provider was unavailable or errored (E6).
    LlmProviderError,
}

/// One source reference inside an Answer, derived from a result in the originating RetrievalRun.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    /// Rank position from the RetrievalRun (1-based).
    pub rank: usize,
    pub chunk_id: ChunkId,
    pub source_path: String,
    pub heading_path: Vec<String>,
    pub preview: String,
}

/// The authoritative record of one answer-generation attempt (answer-generation.md `Answer`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub id: String,
    pub query_id: String,
    pub query_text: String,
    pub retrieval_run_id: String,
    /// Always recorded, even for no-answer responses (FR-RAG-004, AC-4).
    pub prompt_template_version: String,
    /// Empty only when `no_answer_reason` is set to a non-explanatory state; the CLI always shows
    /// a user-facing message (invariant 2, answer-generation.md).
    pub answer_text: String,
    /// MUST be non-empty when `no_answer_reason` is `None` (invariant 1 — see [`Self::invariant_holds`]).
    pub citations: Vec<Citation>,
    /// Retrieved chunks the answer did not cite — for `--show-context` / debugging (FR-RAG-002).
    pub retrieved_but_unused_chunks: Vec<ChunkId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_answer_reason: Option<NoAnswerReason>,
    pub llm_provider: String,
    pub latency_ms: u128,
    pub created_at: String,
}

impl Answer {
    pub fn is_no_answer(&self) -> bool {
        self.no_answer_reason.is_some()
    }

    /// The "Sources Are Mandatory" invariant (PRD §7, FR-RAG-002, AC-7): an Answer without a
    /// no-answer reason must carry at least one Citation. Enforced in code, not by prompt wording.
    pub fn invariant_holds(&self) -> bool {
        self.no_answer_reason.is_some() || !self.citations.is_empty()
    }
}
