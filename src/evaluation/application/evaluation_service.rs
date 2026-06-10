//! EvaluationService — runs each question through Retrieval (via SearchPort), judges relevance,
//! aggregates metrics, applies the threshold gate. Generic over the port → unit-testable with a
//! fake that returns canned RetrievalRuns (no embedder/store/disk).

use crate::evaluation::application::metrics_calc::{compute_metrics, threshold_status};
use crate::evaluation::application::relevance::is_relevant;
use crate::evaluation::domain::question::EvalQuestion;
use crate::evaluation::domain::question_result::EvalQuestionResult;
use crate::evaluation::domain::run::{EvalRun, EvalRunConfig, EvalRunStatus};
use crate::evaluation::ports::SearchPort;
use crate::retrieval::domain::{MetadataFilter, Query};

/// Minimum retrieval depth so Hit@5 + MRR are always computable (ADR-0005, E5).
const MIN_EVAL_K: usize = 5;

pub struct EvaluationService<'a> {
    pub search: &'a dyn SearchPort,
}

impl EvaluationService<'_> {
    /// Run the full dataset. `run_id` and `now` (RFC3339) are injected for determinism.
    pub fn run(
        &self,
        questions: &[EvalQuestion],
        config: &EvalRunConfig,
        dataset_path: &str,
        run_id: &str,
        now: &str,
    ) -> EvalRun {
        let k = config.top_k.max(MIN_EVAL_K);
        let mut results: Vec<EvalQuestionResult> = Vec::with_capacity(questions.len());

        for (i, q) in questions.iter().enumerate() {
            let query = Query {
                text: q.question.clone(),
                mode: config.mode,
                filters: MetadataFilter::default(),
                top_k: k,
                embedding_model: config.embedding_model.clone(),
            };
            match self.search.search(&query) {
                Ok(rrun) => results.push(judge(q, &rrun, &format!("{run_id}_q{i}"))),
                Err(e) => {
                    // E6: a fatal search error (e.g. model mismatch) aborts the run — a config
                    // error must not be reported as a 0% quality regression.
                    return EvalRun {
                        id: run_id.to_string(),
                        dataset_path: dataset_path.to_string(),
                        search_mode: config.mode,
                        top_k: config.top_k,
                        embedding_model: config.embedding_model.clone(),
                        status: EvalRunStatus::Failed,
                        metrics: Default::default(),
                        question_results: results,
                        error: Some(format!("{e:#}")),
                        started_at: now.to_string(),
                        completed_at: now.to_string(),
                    };
                }
            }
        }

        let metrics = compute_metrics(&results);
        let status = threshold_status(&metrics, &config.threshold);
        EvalRun {
            id: run_id.to_string(),
            dataset_path: dataset_path.to_string(),
            search_mode: config.mode,
            top_k: config.top_k,
            embedding_model: config.embedding_model.clone(),
            status,
            metrics,
            question_results: results,
            error: None,
            started_at: now.to_string(),
            completed_at: now.to_string(),
        }
    }
}

/// Score one question against its RetrievalRun (uses first relevant rank only — E7).
fn judge(
    q: &EvalQuestion,
    rrun: &crate::retrieval::domain::RetrievalRun,
    run_id: &str,
) -> EvalQuestionResult {
    let first_rank = rrun
        .results
        .iter()
        .find(|r| is_relevant(q, r))
        .map(|r| r.rank);
    let reciprocal_rank = first_rank.map(|k| 1.0 / k as f64).unwrap_or(0.0);
    EvalQuestionResult {
        question_id: q.id.clone(),
        question_text: q.question.clone(),
        retrieval_run_id: run_id.to_string(),
        search_mode: rrun.search_mode,
        returned_chunk_ids: rrun.results.iter().map(|r| r.chunk_id.clone()).collect(),
        returned_source_paths: rrun.results.iter().map(|r| r.source_path.clone()).collect(),
        hit_at_1: first_rank.is_some_and(|k| k <= 1),
        hit_at_3: first_rank.is_some_and(|k| k <= 3),
        hit_at_5: first_rank.is_some_and(|k| k <= 5),
        reciprocal_rank,
        latency_ms: rrun.latency_ms,
        top_score: rrun.results.first().map(|r| r.score),
        empty: rrun.results.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use crate::ingestion::domain::EmbeddingModelVersion;
    use crate::retrieval::domain::{RetrievalResult, RetrievalRun, RunReason, SearchMode};

    fn model() -> EmbeddingModelVersion {
        EmbeddingModelVersion {
            name: "mock".into(),
            dimension: 8,
            created_at: "t".into(),
        }
    }
    fn config(threshold: Option<f64>) -> EvalRunConfig {
        config_with_mode(SearchMode::Vector, threshold)
    }
    fn config_with_mode(mode: SearchMode, threshold: Option<f64>) -> EvalRunConfig {
        EvalRunConfig {
            mode,
            top_k: 3,
            threshold: crate::evaluation::domain::run::ThresholdConfig {
                min_hit_at_3: threshold,
            },
            embedding_model: model(),
        }
    }
    fn question(id: &str, sources: &[&str]) -> EvalQuestion {
        EvalQuestion {
            id: id.into(),
            question: format!("question {id}"),
            expected_chunk_ids: vec![],
            expected_source_files: sources.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn res(rank: usize, source: &str, score: f32) -> RetrievalResult {
        RetrievalResult {
            rank,
            chunk_id: format!("c{rank}"),
            document_id: "d".into(),
            source_path: source.into(),
            score,
            preview: "p".into(),
            heading_path: vec![],
            metadata: BTreeMap::new(),
        }
    }
    fn run_with(results: Vec<RetrievalResult>) -> RetrievalRun {
        run_with_mode(SearchMode::Vector, results)
    }
    fn run_with_mode(mode: SearchMode, results: Vec<RetrievalResult>) -> RetrievalRun {
        RetrievalRun {
            id: "rr".into(),
            query: Query {
                text: "x".into(),
                mode,
                filters: MetadataFilter::default(),
                top_k: 5,
                embedding_model: model(),
            },
            results,
            search_mode: mode,
            top_k: 5,
            latency_ms: 12,
            below_threshold_count: 0,
            reason: RunReason::Ok,
            explain: None,
            completed_at: "t".into(),
        }
    }

    /// Fake SearchPort: pops a canned RetrievalRun per call, or errors when configured.
    struct FakePort {
        runs: RefCell<std::collections::VecDeque<anyhow::Result<RetrievalRun>>>,
    }
    impl FakePort {
        fn new(runs: Vec<anyhow::Result<RetrievalRun>>) -> Self {
            Self {
                runs: RefCell::new(runs.into_iter().collect()),
            }
        }
    }
    impl SearchPort for FakePort {
        fn search(&self, _q: &Query) -> anyhow::Result<RetrievalRun> {
            self.runs.borrow_mut().pop_front().expect("a canned run")
        }
    }

    #[test]
    fn computes_metrics_over_dataset() {
        let questions = vec![
            question("q1", &["docs/a.md"]),
            question("q2", &["docs/b.md"]),
        ];
        // q1: relevant at rank 1; q2: relevant at rank 2
        let port = FakePort::new(vec![
            Ok(run_with(vec![
                res(1, "docs/a.md", 0.9),
                res(2, "docs/x.md", 0.5),
            ])),
            Ok(run_with(vec![
                res(1, "docs/y.md", 0.7),
                res(2, "docs/b.md", 0.6),
            ])),
        ]);
        let svc = EvaluationService { search: &port };
        let run = svc.run(&questions, &config(None), "ds.json", "ev1", "t");

        assert_eq!(run.status, EvalRunStatus::Completed);
        assert_eq!(run.metrics.question_count, 2);
        assert!((run.metrics.hit_at_1 - 0.5).abs() < 1e-9); // only q1 at rank 1
        assert!((run.metrics.hit_at_3 - 1.0).abs() < 1e-9); // both within 3
                                                            // MRR = (1/1 + 1/2)/2 = 0.75
        assert!((run.metrics.mrr - 0.75).abs() < 1e-9);
        assert_eq!(run.question_results[1].retrieval_run_id, "ev1_q1");
    }

    #[test]
    fn threshold_failure_sets_status_and_no_error() {
        let questions = vec![question("q1", &["docs/a.md"])];
        // miss → hit_at_3 = 0
        let port = FakePort::new(vec![Ok(run_with(vec![res(1, "docs/other.md", 0.9)]))]);
        let svc = EvaluationService { search: &port };
        let run = svc.run(&questions, &config(Some(0.8)), "ds.json", "ev", "t");
        assert_eq!(run.status, EvalRunStatus::ThresholdFailed);
        assert!(run.error.is_none());
        assert_eq!(run.metrics.hit_at_3, 0.0);
    }

    #[test]
    fn fatal_search_error_aborts_with_failed_status() {
        let questions = vec![
            question("q1", &["docs/a.md"]),
            question("q2", &["docs/b.md"]),
        ];
        let port = FakePort::new(vec![Err(anyhow::anyhow!("embedding model mismatch: ..."))]);
        let svc = EvaluationService { search: &port };
        let run = svc.run(&questions, &config(None), "ds.json", "ev", "t");
        assert_eq!(run.status, EvalRunStatus::Failed);
        assert!(run.error.unwrap().contains("mismatch"));
    }

    #[test]
    fn empty_results_count_as_miss_and_empty() {
        let questions = vec![question("q1", &["docs/a.md"])];
        let port = FakePort::new(vec![Ok(run_with(vec![]))]);
        let svc = EvaluationService { search: &port };
        let run = svc.run(&questions, &config(None), "ds.json", "ev", "t");
        assert_eq!(run.metrics.empty_result_count, 1);
        assert_eq!(run.metrics.hit_at_1, 0.0);
        assert_eq!(run.metrics.mrr, 0.0);
    }

    #[test]
    fn keyword_eval_does_not_apply_vector_similarity_threshold() {
        let questions = vec![question("q1", &["docs/a.md"])];
        let port = FakePort::new(vec![Ok(run_with_mode(
            SearchMode::Keyword,
            vec![res(1, "docs/a.md", 0.10)],
        ))]);
        let svc = EvaluationService { search: &port };
        let run = svc.run(
            &questions,
            &config_with_mode(SearchMode::Keyword, None),
            "ds.json",
            "ev",
            "t",
        );
        assert_eq!(run.metrics.empty_result_count, 0);
        assert_eq!(run.metrics.below_threshold_count, 0);
    }
}
