//! M2 integration tests — end-to-end ingest → search through the real ruvector-core + redb
//! adapters, using the deterministic MockEmbedder (no ONNX, no network). Covers the
//! acceptance criteria that depend on the real stack: AC-1/2 (ranked results, source+score),
//! AC-4 (project/tag/source filtering), AC-8 (empty index). Precise ranking/guard logic is
//! unit-tested in src/retrieval/application/search_service.rs with fakes.
//!
//! NOTE: redb and ruvector-core hold an exclusive lock per file, so the ingestion write-handles
//! are dropped (inner scope) before the retrieval read-handles open the same files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use tovli::ingestion::chunking::ChunkingService;
use tovli::ingestion::domain::ChunkingConfig;
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::infra::parsers::default_parsers;
use tovli::ingestion::infra::redb_keyword_index::RedbKeywordIndex;
use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
use tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
use tovli::ingestion::orchestrator::{IngestOptions, IngestionOrchestrator};
use tovli::ingestion::ports::Embedder;

use tovli::retrieval::application::SearchService;
use tovli::retrieval::domain::{MetadataFilter, Query, RunReason, SearchMode};
use tovli::retrieval::infra::redb_keyword_search::RedbKeywordSearch;
use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
use tovli::retrieval::ports::DocumentLookupPort;

const DIM: usize = 16;
const NOW: &str = "2026-06-03T00:00:00Z";
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Paths {
    base: PathBuf,
}
impl Paths {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-m2-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        Self { base }
    }
    fn vectors(&self) -> String {
        self.base.join("vectors.redb").to_string_lossy().to_string()
    }
    fn chunkmap(&self) -> String {
        self.base
            .join("chunkmap.redb")
            .to_string_lossy()
            .to_string()
    }
    fn documents(&self) -> String {
        self.base
            .join("documents.redb")
            .to_string_lossy()
            .to_string()
    }
    fn keyword(&self) -> String {
        self.base.join("keyword.redb").to_string_lossy().to_string()
    }
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), content).unwrap();
}

/// Ingest `dir` with the given project/tags into the shared store at `paths`.
/// Opens and drops the write-handles within this function (releases the file locks).
fn ingest(paths: &Paths, dir: &Path, project: &str, tags: &[&str]) {
    let store = RuVectorStoreAdapter::open(&paths.vectors(), &paths.chunkmap(), DIM).unwrap();
    let docs = RedbDocumentRepository::open(&paths.documents()).unwrap();
    let parsers = default_parsers();
    let emb = MockEmbedder::new(DIM);
    let orch = IngestionOrchestrator {
        parsers: &parsers,
        embedder: &emb,
        store: &store,
        keyword_index: None,
        docs: &docs,
        chunking: ChunkingService::new(ChunkingConfig::default()),
    };
    let opts = IngestOptions {
        dry_run: false,
        force: false,
        project: Some(project.to_string()),
        tags: tags.iter().map(|s| s.to_string()).collect(),
    };
    orch.ingest(dir, &opts, NOW).unwrap();
}

fn ingest_with_keyword(paths: &Paths, dir: &Path, project: &str, tags: &[&str]) {
    let store = RuVectorStoreAdapter::open(&paths.vectors(), &paths.chunkmap(), DIM).unwrap();
    let docs = RedbDocumentRepository::open(&paths.documents()).unwrap();
    let keyword = RedbKeywordIndex::open(&paths.keyword()).unwrap();
    let parsers = default_parsers();
    let emb = MockEmbedder::new(DIM);
    let orch = IngestionOrchestrator {
        parsers: &parsers,
        embedder: &emb,
        store: &store,
        keyword_index: Some(&keyword),
        docs: &docs,
        chunking: ChunkingService::new(ChunkingConfig::default()),
    };
    let opts = IngestOptions {
        dry_run: false,
        force: false,
        project: Some(project.to_string()),
        tags: tags.iter().map(|s| s.to_string()).collect(),
    };
    orch.ingest(dir, &opts, NOW).unwrap();
}

fn query(text: &str, filters: MetadataFilter) -> Query {
    Query {
        text: text.to_string(),
        mode: SearchMode::Vector,
        filters,
        top_k: 8,
        embedding_model: MockEmbedder::new(DIM).model_version().clone(),
    }
}

fn query_mode(text: &str, mode: SearchMode, filters: MetadataFilter) -> Query {
    Query {
        mode,
        ..query(text, filters)
    }
}

#[test]
fn search_returns_ranked_results_with_source_and_score() {
    // AC-1 + AC-2 end-to-end through ruvector-core + redb.
    let paths = Paths::new();
    let dir = paths.base.join("proj_a");
    {
        write(
            &dir,
            "arch.md",
            "# Architecture\n\nlayering rules and component boundaries are important\n",
        );
        write(
            &dir,
            "deploy.md",
            "# Deploy\n\nazure function zipDeploy 403 error during release\n",
        );
        ingest(&paths, &dir, "flexid", &["arch"]);
    } // write-handles dropped here

    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let indexed = lookup
        .indexed_model_version()
        .unwrap()
        .expect("index has a model");
    assert_eq!(indexed.dimension, DIM);
    let store = RuVectorSearchAdapter::open(&paths.vectors(), indexed.dimension).unwrap();
    let keyword = RedbKeywordSearch::open(&paths.keyword()).unwrap();
    let svc = SearchService {
        embedder: &MockEmbedder::new(DIM),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };

    let run = svc
        .search(
            &query("architecture boundaries", MetadataFilter::default()),
            false,
            "rrun_it",
            NOW,
        )
        .unwrap();

    assert_eq!(run.reason, RunReason::Ok);
    assert!(!run.results.is_empty(), "expected at least one result");
    // every result carries a source file and a similarity score in [0,1]  (AC-2)
    for r in &run.results {
        assert!(
            r.source_path.ends_with(".md"),
            "source present: {}",
            r.source_path
        );
        assert!(
            (0.0..=1.0).contains(&r.score),
            "score in range: {}",
            r.score
        );
    }
    // scores are non-increasing by rank  (AC-2)
    for w in run.results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "scores non-increasing: {} then {}",
            w[0].score,
            w[1].score
        );
    }
    assert_eq!(run.results[0].rank, 1);
}

#[test]
fn project_and_tag_and_source_filters_apply() {
    // AC-4 — two projects in one index; filter isolates one.
    let paths = Paths::new();
    let dir_a = paths.base.join("proj_a");
    let dir_b = paths.base.join("proj_b");
    {
        write(
            &dir_a,
            "arch.md",
            "# Architecture\n\nlayering rules and boundaries\n",
        );
        write(
            &dir_b,
            "ops.md",
            "# Ops\n\nincident runbook for the gateway\n",
        );
        ingest(&paths, &dir_a, "flexid", &["arch"]);
        ingest(&paths, &dir_b, "ops", &["ops"]);
    }

    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let indexed = lookup.indexed_model_version().unwrap().unwrap();
    let store = RuVectorSearchAdapter::open(&paths.vectors(), indexed.dimension).unwrap();
    let keyword = RedbKeywordSearch::open(&paths.keyword()).unwrap();
    let svc = SearchService {
        embedder: &MockEmbedder::new(DIM),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };

    // no filter → results from both projects
    let all = svc
        .search(&query("rules", MetadataFilter::default()), false, "r", NOW)
        .unwrap();
    assert!(
        all.results.len() >= 2,
        "both projects' chunks should be searchable"
    );

    // project filter → only flexid (arch.md)
    let f = MetadataFilter {
        project: Some("flexid".into()),
        ..Default::default()
    };
    let only_flexid = svc.search(&query("rules", f), false, "r", NOW).unwrap();
    assert!(!only_flexid.results.is_empty());
    assert!(only_flexid
        .results
        .iter()
        .all(|r| r.source_path.ends_with("arch.md")));

    // tag filter → only ops
    let f = MetadataFilter {
        tags: vec!["ops".into()],
        ..Default::default()
    };
    let only_ops = svc.search(&query("rules", f), false, "r", NOW).unwrap();
    assert!(!only_ops.results.is_empty());
    assert!(only_ops
        .results
        .iter()
        .all(|r| r.source_path.ends_with("ops.md")));

    // source filter → exact file
    let src = only_flexid.results[0].source_path.clone();
    let f = MetadataFilter {
        source: Some(src.clone()),
        ..Default::default()
    };
    let only_src = svc.search(&query("rules", f), false, "r", NOW).unwrap();
    assert!(only_src.results.iter().all(|r| r.source_path == src));

    // filter that matches nothing → Ok run, empty results (AC-5)
    let f = MetadataFilter {
        project: Some("nonexistent".into()),
        ..Default::default()
    };
    let none = svc.search(&query("rules", f), false, "r", NOW).unwrap();
    assert_eq!(none.reason, RunReason::Ok);
    assert!(none.results.is_empty());
}

#[test]
fn empty_index_reports_index_empty() {
    // AC-8 — nothing ingested; documents.redb does not exist.
    let paths = Paths::new();
    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    assert!(
        lookup.indexed_model_version().unwrap().is_none(),
        "no model on an empty index"
    );
}

#[test]
fn keyword_and_hybrid_search_use_the_real_keyword_index() {
    let paths = Paths::new();
    let dir = paths.base.join("docs");
    {
        write(
            &dir,
            "arch.md",
            "# Architecture\n\nlayering rules and component boundaries\n",
        );
        write(
            &dir,
            "deploy.md",
            "# Deploy\n\nAzure Function zipDeploy 403 error during release\n",
        );
        ingest_with_keyword(&paths, &dir, "flexid", &["ops"]);
    }

    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let indexed = lookup.indexed_model_version().unwrap().unwrap();
    let store = RuVectorSearchAdapter::open(&paths.vectors(), indexed.dimension).unwrap();
    let keyword = RedbKeywordSearch::open(&paths.keyword()).unwrap();
    let svc = SearchService {
        embedder: &MockEmbedder::new(DIM),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };

    let keyword_run = svc
        .search(
            &query_mode(
                "zipDeploy 403",
                SearchMode::Keyword,
                MetadataFilter::default(),
            ),
            true,
            "rrun_kw",
            NOW,
        )
        .unwrap();
    assert_eq!(keyword_run.search_mode, SearchMode::Keyword);
    assert!(keyword_run.results[0].source_path.ends_with("deploy.md"));
    assert_eq!(
        keyword_run.explain.as_ref().unwrap().ranking_method,
        "keyword"
    );
    assert_eq!(
        keyword_run.explain.as_ref().unwrap().result_details[0].vector_score,
        None
    );

    let hybrid_run = svc
        .search(
            &query_mode(
                "zipDeploy 403",
                SearchMode::Hybrid,
                MetadataFilter::default(),
            ),
            true,
            "rrun_hybrid",
            NOW,
        )
        .unwrap();
    assert_eq!(hybrid_run.search_mode, SearchMode::Hybrid);
    assert_eq!(hybrid_run.explain.as_ref().unwrap().ranking_method, "rrf");
    assert!(hybrid_run
        .explain
        .as_ref()
        .unwrap()
        .result_details
        .iter()
        .any(|d| d.keyword_score.is_some() && d.fused_score > 0.0));
}
