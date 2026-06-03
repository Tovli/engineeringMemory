//! M3 integration tests — end-to-end ingest → evaluate through the real retrieval stack, using
//! the deterministic MockEmbedder (no ONNX). With a tiny corpus the top-5 search returns every
//! chunk, so a question whose expected source file was ingested is a guaranteed hit — this makes
//! Hit@K deterministic without semantic embeddings (ADR-0005). Metric math + relevance + threshold
//! are unit-tested in src/evaluation/. Semantic Hit@3 ≥ 0.80 is verified locally with ONNX.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use tovli::ingestion::chunking::ChunkingService;
use tovli::ingestion::domain::ChunkingConfig;
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::infra::parsers::default_parsers;
use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
use tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
use tovli::ingestion::orchestrator::{IngestOptions, IngestionOrchestrator};
use tovli::ingestion::ports::Embedder;

use tovli::retrieval::application::SearchService;
use tovli::retrieval::domain::SearchMode;
use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
use tovli::retrieval::ports::DocumentLookupPort;

use tovli::evaluation::application::EvaluationService;
use tovli::evaluation::domain::question::EvalQuestion;
use tovli::evaluation::domain::{EvalRunConfig, EvalRunStatus, ThresholdConfig};
use tovli::evaluation::infra::report_writer::write_report;
use tovli::evaluation::infra::retrieval_search_adapter::RetrievalSearchAdapter;

const DIM: usize = 16;
const NOW: &str = "2026-06-03T00:00:00Z";
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Paths {
    base: PathBuf,
}
impl Paths {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-m3-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        Self { base }
    }
    fn vectors(&self) -> String {
        self.base.join("vectors.redb").to_string_lossy().to_string()
    }
    fn chunkmap(&self) -> String {
        self.base.join("chunkmap.redb").to_string_lossy().to_string()
    }
    fn documents(&self) -> String {
        self.base.join("documents.redb").to_string_lossy().to_string()
    }
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
}

/// Ingest `dir` into the shared store; opens and drops write-handles here (releases file locks).
fn ingest(paths: &Paths, dir: &Path) {
    let store = RuVectorStoreAdapter::open(&paths.vectors(), &paths.chunkmap(), DIM).unwrap();
    let docs = RedbDocumentRepository::open(&paths.documents()).unwrap();
    let parsers = default_parsers();
    let emb = MockEmbedder::new(DIM);
    let orch = IngestionOrchestrator {
        parsers: &parsers,
        embedder: &emb,
        store: &store,
        docs: &docs,
        chunking: ChunkingService::new(ChunkingConfig::default()),
    };
    orch.ingest(dir, &IngestOptions::default(), NOW).unwrap();
}

fn question(id: &str, source: &str) -> EvalQuestion {
    EvalQuestion {
        id: id.into(),
        question: format!("tell me about {source}"),
        expected_chunk_ids: vec![],
        expected_source_files: vec![source.into()],
    }
}

fn config(threshold: Option<f64>) -> EvalRunConfig {
    EvalRunConfig {
        mode: SearchMode::Vector,
        top_k: 5,
        threshold: ThresholdConfig { min_hit_at_3: threshold },
        embedding_model: MockEmbedder::new(DIM).model_version().clone(),
    }
}

#[test]
fn end_to_end_eval_computes_metrics_and_writes_report() {
    // AC-1..AC-5: dataset runs, Hit@K/MRR/latency computed, JSON report written.
    let paths = Paths::new();
    let dir = paths.base.join("docs");
    {
        write(&dir, "arch.md", "# Architecture\n\nlayering rules and component boundaries\n");
        write(&dir, "deploy.md", "# Deploy\n\nazure function zipDeploy 403 during release\n");
        ingest(&paths, &dir);
    }

    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let dim = lookup.indexed_model_version().unwrap().unwrap().dimension;
    let store = RuVectorSearchAdapter::open(&paths.vectors(), dim).unwrap();
    let search = SearchService { embedder: &MockEmbedder::new(DIM), store: &store, lookup: &lookup };
    let adapter = RetrievalSearchAdapter { inner: search };
    let evaluator = EvaluationService { search: &adapter };

    // Both files are ingested and only 2 chunks exist, so top-5 returns both → every question hits.
    let questions = vec![question("q1", "docs/arch.md"), question("q2", "docs/deploy.md")];
    let run = evaluator.run(&questions, &config(Some(0.8)), "ds.json", "ev_it", NOW);

    assert_eq!(run.status, EvalRunStatus::Completed, "Hit@3 should clear 0.8");
    assert_eq!(run.metrics.question_count, 2);
    assert_eq!(run.metrics.hit_at_3, 1.0);
    assert_eq!(run.metrics.hit_at_5, 1.0);
    assert!(run.metrics.mrr > 0.0);
    assert_eq!(run.metrics.empty_result_count, 0);

    // AC-5: report written and re-readable with the right shape.
    let out = paths.base.join("report.json").to_string_lossy().to_string();
    write_report(&out, &run).unwrap();
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(report["metrics"]["questionCount"], 2);
    assert_eq!(report["searchMode"], "vector");
    assert_eq!(report["questionResults"].as_array().unwrap().len(), 2);
    assert!(report["metrics"]["hitAt3"].as_f64().unwrap() >= 0.8);
}

#[test]
fn threshold_failure_when_expected_file_absent() {
    // AC-6: a question pointing at a never-ingested file misses → Hit@3 = 0 → ThresholdFailed.
    let paths = Paths::new();
    let dir = paths.base.join("docs");
    {
        write(&dir, "arch.md", "# Architecture\n\nlayering rules\n");
        ingest(&paths, &dir);
    }
    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let dim = lookup.indexed_model_version().unwrap().unwrap().dimension;
    let store = RuVectorSearchAdapter::open(&paths.vectors(), dim).unwrap();
    let search = SearchService { embedder: &MockEmbedder::new(DIM), store: &store, lookup: &lookup };
    let adapter = RetrievalSearchAdapter { inner: search };
    let evaluator = EvaluationService { search: &adapter };

    let questions = vec![question("q1", "docs/never-ingested.md")];
    let run = evaluator.run(&questions, &config(Some(0.8)), "ds.json", "ev", NOW);

    assert_eq!(run.status, EvalRunStatus::ThresholdFailed);
    assert_eq!(run.metrics.hit_at_3, 0.0);
    assert!(run.error.is_none(), "threshold miss is not a fatal error");
}
