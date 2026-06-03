//! EvalQuestion — one ground-truth test case (PRD §8.7 FR-EVAL-001).

use serde::{Deserialize, Serialize};

use crate::ingestion::domain::ChunkId;

/// A single test case loaded from the dataset JSON. At least one of
/// `expected_chunk_ids` / `expected_source_files` must be non-empty (validated at load).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalQuestion {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub expected_chunk_ids: Vec<ChunkId>,
    #[serde(default)]
    pub expected_source_files: Vec<String>,
}

impl EvalQuestion {
    /// EvalQuestion invariant 1 (evaluation.md): some ground truth must exist.
    pub fn has_ground_truth(&self) -> bool {
        !self.expected_chunk_ids.is_empty() || !self.expected_source_files.is_empty()
    }
}
