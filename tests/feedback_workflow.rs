//! M6 feedback tests: append-only ratings tied to retrieval-run evidence, plus report/export
//! aggregation. These tests drive ADR-0011.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use std::cell::{Cell, RefCell};

use tovli::feedback::application::{next_feedback_ids, next_query_run_ids};
use tovli::feedback::application::{FeedbackReportService, FeedbackService, RecordFeedbackInput};
use tovli::feedback::domain::{
    FeedbackItem, FeedbackQuery, FeedbackRating, RetrievalRunEvidence, RetrievedChunkEvidence,
};
use tovli::feedback::infra::{export_feedback_json, JsonlRetrievalRunLog, RedbFeedbackRepository};
use tovli::feedback::ports::FeedbackRepository;
use tovli::retrieval::domain::SearchMode;

const NOW: &str = "2026-06-10T00:00:00Z";
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Paths {
    base: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-m6-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        Self { base }
    }

    fn feedback(&self) -> String {
        self.base
            .join("feedback.redb")
            .to_string_lossy()
            .to_string()
    }

    fn runs(&self) -> String {
        self.base
            .join("retrieval-runs.jsonl")
            .to_string_lossy()
            .to_string()
    }

    fn export(&self) -> String {
        self.base
            .join("feedback.json")
            .to_string_lossy()
            .to_string()
    }
}

fn chunk(
    chunk_id: &str,
    document_id: &str,
    source_path: &str,
    rank: usize,
    score: f32,
) -> RetrievedChunkEvidence {
    RetrievedChunkEvidence {
        rank,
        chunk_id: chunk_id.into(),
        document_id: document_id.into(),
        source_path: source_path.into(),
        score,
        preview: format!("preview for {chunk_id}"),
        heading_path: vec!["Heading".into()],
    }
}

fn evidence(
    retrieval_run_id: &str,
    query_id: &str,
    question_text: &str,
    results: Vec<RetrievedChunkEvidence>,
) -> RetrievalRunEvidence {
    RetrievalRunEvidence {
        retrieval_run_id: retrieval_run_id.into(),
        query_id: query_id.into(),
        question_text: question_text.into(),
        search_mode: SearchMode::Hybrid,
        top_k: 8,
        created_at: NOW.into(),
        results,
    }
}

#[test]
fn records_feedback_only_for_chunks_displayed_in_the_retrieval_run() {
    let paths = Paths::new();
    let runs = JsonlRetrievalRunLog::open(&paths.runs());
    let repo = RedbFeedbackRepository::open(&paths.feedback()).unwrap();
    let service = FeedbackService {
        feedback: &repo,
        runs: &runs,
    };

    runs.append(&evidence(
        "rrun_1",
        "qry_1",
        "why did deployment fail?",
        vec![chunk("chunk_good", "doc_a", "docs/deploy.md", 1, 0.91)],
    ))
    .unwrap();

    let saved = service
        .record(RecordFeedbackInput {
            feedback_id: "fb_1".into(),
            query_id: "qry_1".into(),
            retrieval_run_id: "rrun_1".into(),
            chunk_id: "chunk_good".into(),
            rating: FeedbackRating::Good,
            note: Some("grounded answer".into()),
            created_at: NOW.into(),
        })
        .unwrap();

    assert_eq!(saved.query_id, "qry_1");
    assert_eq!(saved.retrieval_run_id, "rrun_1");
    assert_eq!(saved.chunk_id, "chunk_good");
    assert_eq!(saved.search_mode, SearchMode::Hybrid);
    assert_eq!(saved.rank, 1);
    assert_eq!(saved.source_path, "docs/deploy.md");

    service
        .record(RecordFeedbackInput {
            feedback_id: "fb_2".into(),
            query_id: "qry_1".into(),
            retrieval_run_id: "rrun_1".into(),
            chunk_id: "chunk_good".into(),
            rating: FeedbackRating::Bad,
            note: Some("correction is append-only".into()),
            created_at: "2026-06-10T00:01:00Z".into(),
        })
        .unwrap();
    assert_eq!(repo.find_all(None).unwrap().len(), 2);

    let err = service
        .record(RecordFeedbackInput {
            feedback_id: "fb_3".into(),
            query_id: "qry_1".into(),
            retrieval_run_id: "rrun_1".into(),
            chunk_id: "chunk_not_shown".into(),
            rating: FeedbackRating::Bad,
            note: None,
            created_at: NOW.into(),
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("was not displayed"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn report_surfaces_problem_queries_downvoted_chunks_no_good_results_and_rechunking_candidates() {
    let paths = Paths::new();
    let runs = JsonlRetrievalRunLog::open(&paths.runs());
    let repo = RedbFeedbackRepository::open(&paths.feedback()).unwrap();
    let service = FeedbackService {
        feedback: &repo,
        runs: &runs,
    };

    runs.append(&evidence(
        "rrun_a",
        "qry_a",
        "zipDeploy 403",
        vec![
            chunk("chunk_bad_a", "doc_deploy", "docs/deploy.md", 1, 0.84),
            chunk("chunk_good_a", "doc_deploy", "docs/deploy.md", 2, 0.80),
        ],
    ))
    .unwrap();
    runs.append(&evidence(
        "rrun_b",
        "qry_b",
        "function release auth error",
        vec![chunk(
            "chunk_bad_a",
            "doc_deploy",
            "docs/deploy.md",
            1,
            0.78,
        )],
    ))
    .unwrap();

    for (id, query, run, chunk_id, rating) in [
        (
            "fb_bad_1",
            "qry_a",
            "rrun_a",
            "chunk_bad_a",
            FeedbackRating::Bad,
        ),
        (
            "fb_bad_2",
            "qry_a",
            "rrun_a",
            "chunk_good_a",
            FeedbackRating::Bad,
        ),
        (
            "fb_good_1",
            "qry_a",
            "rrun_a",
            "chunk_good_a",
            FeedbackRating::Good,
        ),
        (
            "fb_bad_3",
            "qry_b",
            "rrun_b",
            "chunk_bad_a",
            FeedbackRating::Bad,
        ),
    ] {
        service
            .record(RecordFeedbackInput {
                feedback_id: id.into(),
                query_id: query.into(),
                retrieval_run_id: run.into(),
                chunk_id: chunk_id.into(),
                rating,
                note: None,
                created_at: NOW.into(),
            })
            .unwrap();
    }

    let report = FeedbackReportService { feedback: &repo }
        .generate("rpt_1", NOW)
        .unwrap();

    let query_a = report
        .problematic_queries
        .iter()
        .find(|q| q.query_id == "qry_a")
        .expect("qry_a should be problematic");
    assert_eq!(query_a.bad_count, 2);
    assert_eq!(query_a.good_count, 1);
    assert!((query_a.bad_ratio - 2.0 / 3.0).abs() < f64::EPSILON);

    let downvoted = &report.frequently_downvoted_chunks[0];
    assert_eq!(downvoted.chunk_id, "chunk_bad_a");
    assert_eq!(downvoted.bad_count, 2);
    assert_eq!(downvoted.distinct_query_count, 2);

    assert!(report
        .queries_with_no_good_result
        .iter()
        .any(|q| q.query_id == "qry_b" && q.good_count == 0));
    assert!(report.observations.iter().any(|o| {
        o.feedback_id == "fb_bad_1"
            && o.retrieval_run_id == "rrun_a"
            && o.rank == 1
            && (o.score - 0.84).abs() < f32::EPSILON
            && o.search_mode == SearchMode::Hybrid
    }));
    assert!(report
        .candidates_for_rechunking
        .iter()
        .any(|c| c.document_id == "doc_deploy" && c.downvoted_chunk_count >= 2));

    export_feedback_json(&paths.export(), &repo).unwrap();
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(paths.export()).unwrap()).unwrap();
    assert_eq!(exported.as_array().unwrap().len(), 4);
    assert_eq!(exported[0]["queryId"], "qry_a");
}

#[test]
fn generated_query_and_run_ids_are_unique_within_the_same_millisecond() {
    let first = next_query_run_ids(1_780_000_000_000);
    let second = next_query_run_ids(1_780_000_000_000);
    let expected_query_prefix = format!("qry_1780000000000_{}_", std::process::id());
    let expected_run_prefix = format!("rrun_1780000000000_{}_", std::process::id());

    assert!(first.query_id.starts_with(&expected_query_prefix));
    assert!(first.retrieval_run_id.starts_with(&expected_run_prefix));
    assert_eq!(
        first.query_id.strip_prefix("qry_"),
        first.retrieval_run_id.strip_prefix("rrun_")
    );
    assert_ne!(first.query_id, second.query_id);
    assert_ne!(first.retrieval_run_id, second.retrieval_run_id);
}

#[test]
fn generated_feedback_ids_include_timestamp_pid_sequence_and_batch_index() {
    let ids = next_feedback_ids(1_780_000_000_000_000_000, 2);
    let expected_prefix = format!("fb_1780000000000000000_{}_", std::process::id());

    assert_eq!(ids.len(), 2);
    assert!(ids[0].starts_with(&expected_prefix));
    assert!(ids[1].starts_with(&expected_prefix));
    assert!(ids[0].ends_with("_0"));
    assert!(ids[1].ends_with("_1"));
    assert_ne!(ids[0], ids[1]);
}

fn feedback_item(id: &str) -> FeedbackItem {
    FeedbackItem {
        id: id.into(),
        query_id: "qry_1".into(),
        retrieval_run_id: "rrun_1".into(),
        chunk_id: "chunk_1".into(),
        document_id: "doc_1".into(),
        rating: FeedbackRating::Good,
        note: None,
        search_mode: SearchMode::Hybrid,
        rank: 1,
        score: 0.91,
        source_path: "docs/deploy.md".into(),
        question_text: "why did deployment fail?".into(),
        created_at: NOW.into(),
    }
}

#[test]
fn redb_batch_conflict_does_not_persist_earlier_new_item() {
    let paths = Paths::new();
    let repo = RedbFeedbackRepository::open(&paths.feedback()).unwrap();
    let existing = feedback_item("fb_existing");
    let new_item = feedback_item("fb_new");

    repo.save(&existing).unwrap();
    let err = repo.save_many(&[new_item, existing.clone()]).unwrap_err();

    assert!(err.to_string().contains("fb_existing"));
    assert_eq!(repo.find_all(None).unwrap(), vec![existing]);
}

struct FailingBatchRepository {
    saved: RefCell<Vec<FeedbackItem>>,
    save_calls: Cell<usize>,
}

impl FailingBatchRepository {
    fn new() -> Self {
        Self {
            saved: RefCell::new(Vec::new()),
            save_calls: Cell::new(0),
        }
    }
}

impl FeedbackRepository for FailingBatchRepository {
    fn save(&self, item: &FeedbackItem) -> anyhow::Result<()> {
        let calls = self.save_calls.get();
        self.save_calls.set(calls + 1);
        if calls == 1 {
            anyhow::bail!("simulated second-write failure");
        }
        self.saved.borrow_mut().push(item.clone());
        Ok(())
    }

    fn save_many(&self, items: &[FeedbackItem]) -> anyhow::Result<()> {
        self.save_calls.set(self.save_calls.get() + items.len());
        anyhow::bail!("simulated second-write failure")
    }

    fn find_by_id(&self, _id: &str) -> anyhow::Result<Option<FeedbackItem>> {
        Ok(None)
    }

    fn find_all(&self, _query: Option<FeedbackQuery>) -> anyhow::Result<Vec<FeedbackItem>> {
        Ok(self.saved.borrow().clone())
    }
}

#[test]
fn record_many_does_not_partially_persist_when_repository_batch_save_fails() {
    let paths = Paths::new();
    let runs = JsonlRetrievalRunLog::open(&paths.runs());
    runs.append(&evidence(
        "rrun_1",
        "qry_1",
        "why did deployment fail?",
        vec![
            chunk("chunk_a", "doc_a", "docs/deploy.md", 1, 0.91),
            chunk("chunk_b", "doc_a", "docs/deploy.md", 2, 0.80),
        ],
    ))
    .unwrap();

    let repo = FailingBatchRepository::new();
    let service = FeedbackService {
        feedback: &repo,
        runs: &runs,
    };

    let err = service
        .record_many(vec![
            RecordFeedbackInput {
                feedback_id: "fb_1".into(),
                query_id: "qry_1".into(),
                retrieval_run_id: "rrun_1".into(),
                chunk_id: "chunk_a".into(),
                rating: FeedbackRating::Good,
                note: None,
                created_at: NOW.into(),
            },
            RecordFeedbackInput {
                feedback_id: "fb_2".into(),
                query_id: "qry_1".into(),
                retrieval_run_id: "rrun_1".into(),
                chunk_id: "chunk_b".into(),
                rating: FeedbackRating::Bad,
                note: None,
                created_at: NOW.into(),
            },
        ])
        .unwrap_err();

    assert!(err.to_string().contains("simulated second-write failure"));
    assert_eq!(repo.saved.borrow().len(), 0);
}
