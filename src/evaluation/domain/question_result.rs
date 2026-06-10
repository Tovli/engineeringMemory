//! EvalQuestionResult — per-question outcome (PRD §8.7, evaluation.md). Report + debug artefact.

use serde::{Deserialize, Serialize};

use crate::ingestion::domain::ChunkId;
use crate::retrieval::domain::SearchMode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalQuestionResult {
    pub question_id: String,
    pub question_text: String,
    pub retrieval_run_id: String,
    pub search_mode: SearchMode,
    pub returned_chunk_ids: Vec<ChunkId>,
    pub returned_source_paths: Vec<String>,
    pub hit_at_1: bool,
    pub hit_at_3: bool,
    pub hit_at_5: bool,
    /// 1/rank of the first relevant result; 0.0 if none was relevant.
    pub reciprocal_rank: f64,
    pub latency_ms: u128,
    pub top_score: Option<f32>,
    pub empty: bool,
}
