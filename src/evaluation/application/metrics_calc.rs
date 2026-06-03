//! MetricsCalculationService — pure aggregation of per-question results (FR-EVAL-002).
//! No I/O; directly unit-testable.

use crate::evaluation::domain::metrics::EvalMetrics;
use crate::evaluation::domain::question_result::EvalQuestionResult;
use crate::evaluation::domain::run::{EvalRunStatus, ThresholdConfig};
use crate::retrieval::application::scoring::SIMILARITY_THRESHOLD;

/// Compute aggregate metrics from per-question results.
pub fn compute_metrics(results: &[EvalQuestionResult]) -> EvalMetrics {
    let n = results.len();
    if n == 0 {
        return EvalMetrics::default();
    }
    let nf = n as f64;
    let frac = |count: usize| count as f64 / nf;
    let hits = |pred: fn(&EvalQuestionResult) -> bool| results.iter().filter(|r| pred(r)).count();

    let below = results
        .iter()
        .filter(|r| match r.top_score {
            None => true,
            Some(s) => s < SIMILARITY_THRESHOLD,
        })
        .count();

    EvalMetrics {
        hit_at_1: frac(hits(|r| r.hit_at_1)),
        hit_at_3: frac(hits(|r| r.hit_at_3)),
        hit_at_5: frac(hits(|r| r.hit_at_5)),
        mrr: results.iter().map(|r| r.reciprocal_rank).sum::<f64>() / nf,
        avg_latency_ms: results.iter().map(|r| r.latency_ms as f64).sum::<f64>() / nf,
        empty_result_count: results.iter().filter(|r| r.empty).count(),
        below_threshold_count: below,
        question_count: n,
    }
}

/// Decide pass/fail against the threshold (R7, AC-6, E8: equality passes).
pub fn threshold_status(metrics: &EvalMetrics, threshold: &ThresholdConfig) -> EvalRunStatus {
    if let Some(min) = threshold.min_hit_at_3 {
        if metrics.hit_at_3 < min {
            return EvalRunStatus::ThresholdFailed;
        }
    }
    EvalRunStatus::Completed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(hit_rank: Option<usize>, latency: u128, top: Option<f32>) -> EvalQuestionResult {
        let rr = hit_rank.map(|k| 1.0 / k as f64).unwrap_or(0.0);
        EvalQuestionResult {
            question_id: "q".into(),
            question_text: "?".into(),
            retrieval_run_id: "r".into(),
            returned_chunk_ids: vec![],
            returned_source_paths: vec![],
            hit_at_1: hit_rank.is_some_and(|k| k <= 1),
            hit_at_3: hit_rank.is_some_and(|k| k <= 3),
            hit_at_5: hit_rank.is_some_and(|k| k <= 5),
            reciprocal_rank: rr,
            latency_ms: latency,
            top_score: top,
            empty: top.is_none(),
        }
    }

    #[test]
    fn empty_results_zeroed() {
        let m = compute_metrics(&[]);
        assert_eq!(m, EvalMetrics::default());
    }

    #[test]
    fn computes_hits_mrr_latency() {
        // q1 hit@1 (rank1), q2 hit@3 (rank2), q3 miss
        let results = vec![
            r(Some(1), 10, Some(0.9)),
            r(Some(2), 20, Some(0.8)),
            r(None, 30, Some(0.1)),
        ];
        let m = compute_metrics(&results);
        assert_eq!(m.question_count, 3);
        assert!((m.hit_at_1 - 1.0 / 3.0).abs() < 1e-9);
        assert!((m.hit_at_3 - 2.0 / 3.0).abs() < 1e-9);
        assert!((m.hit_at_5 - 2.0 / 3.0).abs() < 1e-9);
        // MRR = (1/1 + 1/2 + 0) / 3 = 0.5
        assert!((m.mrr - 0.5).abs() < 1e-9);
        assert!((m.avg_latency_ms - 20.0).abs() < 1e-9);
        // q3 top_score 0.1 < 0.30 threshold → below; q1/q2 above
        assert_eq!(m.below_threshold_count, 1);
        assert_eq!(m.empty_result_count, 0);
    }

    #[test]
    fn empty_and_below_threshold_counts() {
        let results = vec![r(None, 5, None), r(Some(1), 5, Some(0.95))];
        let m = compute_metrics(&results);
        assert_eq!(m.empty_result_count, 1); // top_score None → empty
        assert_eq!(m.below_threshold_count, 1); // the empty one counts as below
    }

    #[test]
    fn threshold_gate_strict_below_fails_equal_passes() {
        let mut m = EvalMetrics { hit_at_3: 0.80, ..Default::default() };
        let t = ThresholdConfig { min_hit_at_3: Some(0.80) };
        assert_eq!(threshold_status(&m, &t), EvalRunStatus::Completed); // equal passes (E8)
        m.hit_at_3 = 0.79;
        assert_eq!(threshold_status(&m, &t), EvalRunStatus::ThresholdFailed);
        // no threshold configured → always completed
        assert_eq!(threshold_status(&m, &ThresholdConfig::default()), EvalRunStatus::Completed);
    }
}
