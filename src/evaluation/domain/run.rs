//! EvalRun aggregate root + config/threshold value objects (evaluation.md).

use serde::{Deserialize, Serialize};

use crate::ingestion::domain::EmbeddingModelVersion;
use crate::evaluation::domain::metrics::EvalMetrics;
use crate::evaluation::domain::question_result::EvalQuestionResult;
use crate::retrieval::domain::SearchMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStatus {
    /// All questions ran; metrics computed; threshold (if any) satisfied.
    Completed,
    /// All questions ran but Hit@3 fell below the configured threshold (CI gate, FR-EVAL-003).
    ThresholdFailed,
    /// A question's search failed fatally (e.g. EmbeddingModelMismatch) — run aborted (E6).
    Failed,
}

/// CI regression gate (PRD §8.7 FR-EVAL-003).
#[derive(Debug, Clone, Copy, Default)]
pub struct ThresholdConfig {
    pub min_hit_at_3: Option<f64>,
}

/// Inputs for one evaluation run.
#[derive(Debug, Clone)]
pub struct EvalRunConfig {
    pub mode: SearchMode,
    pub top_k: usize,
    pub threshold: ThresholdConfig,
    pub embedding_model: EmbeddingModelVersion,
}

/// The authoritative record of one evaluation execution.
#[derive(Debug, Clone)]
pub struct EvalRun {
    pub id: String,
    pub dataset_path: String,
    pub search_mode: SearchMode,
    pub top_k: usize,
    pub embedding_model: EmbeddingModelVersion,
    pub status: EvalRunStatus,
    pub metrics: EvalMetrics,
    pub question_results: Vec<EvalQuestionResult>,
    /// Set only when `status == Failed`.
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: String,
}
