//! EvalMetrics — aggregate retrieval-quality metrics (PRD §8.7 FR-EVAL-002).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalMetrics {
    /// Fraction of questions with a relevant result at rank ≤ 1 (in [0,1]).
    pub hit_at_1: f64,
    pub hit_at_3: f64,
    pub hit_at_5: f64,
    /// Mean Reciprocal Rank (in [0,1]).
    pub mrr: f64,
    pub avg_latency_ms: f64,
    /// Questions that returned zero results.
    pub empty_result_count: usize,
    /// Questions with no result, plus vector-mode questions whose top result is below
    /// SIMILARITY_THRESHOLD.
    pub below_threshold_count: usize,
    pub question_count: usize,
}
