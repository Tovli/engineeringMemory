//! M4 integration test — end-to-end ingest → retrieve → generate a cited answer through the real
//! retrieval stack with the deterministic MockEmbedder + MockLlm (no ONNX, no network).
//!
//! Determinism trick (ADR-0005 reused): the MockEmbedder hashes the exact chunk text, so a query
//! whose text *equals* a chunk's content embeds identically → cosine distance ≈ 0 → similarity ≈ 1.0,
//! which clears SIMILARITY_THRESHOLD and makes that chunk the grounding source. The MockLlm then
//! cites the `[[chunk:...]]` tag the renderer emitted, so the answer is grounded and deterministic.
//! All no-answer branches are unit-tested in src/answer_generation/application/rag_service.rs.

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
use tovli::retrieval::domain::{MetadataFilter, Query, SearchMode};
use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
use tovli::retrieval::ports::DocumentLookupPort;

use tovli::answer_generation::application::rag_service::{AnswerContext, RagAnswerService};
use tovli::answer_generation::infra::answer_log_writer::append_answer_log;
use tovli::answer_generation::infra::mock_llm::MockLlm;

const DIM: usize = 16;
const NOW: &str = "2026-06-06T00:00:00Z";
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Paths {
    base: PathBuf,
}
impl Paths {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-m4-{}-{}", std::process::id(), n));
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

fn query(text: &str) -> Query {
    Query {
        text: text.into(),
        mode: SearchMode::Vector,
        filters: MetadataFilter::default(),
        top_k: 8,
        embedding_model: MockEmbedder::new(DIM).model_version().clone(),
    }
}

#[test]
fn end_to_end_ask_generates_a_cited_grounded_answer() {
    // AC-1: ask over an ingested corpus → grounded answer with at least one citation.
    let paths = Paths::new();
    let dir = paths.base.join("docs");
    // Single heading + single paragraph → the chunk content is exactly the paragraph text.
    let body = "layering rules and component boundaries";
    write(&dir, "arch.md", &format!("# Architecture\n\n{body}\n"));
    ingest(&paths, &dir);

    let lookup = RedbDocumentLookup::open(&paths.documents()).unwrap();
    let dim = lookup.indexed_model_version().unwrap().unwrap().dimension;
    let store = RuVectorSearchAdapter::open(&paths.vectors(), dim).unwrap();
    let search = SearchService { embedder: &MockEmbedder::new(DIM), store: &store, lookup: &lookup };

    // Query text == chunk content → mock embeds identically → similarity ≈ 1.0 clears threshold.
    let run = search.search(&query(body), false, "rrun_it", NOW).unwrap();
    assert!(!run.results.is_empty(), "the ingested chunk should be retrieved");

    let llm = MockLlm::default();
    let rag = RagAnswerService { llm: &llm };
    let actx = AnswerContext { query_id: "qry_it", answer_id: "ans_it", now: NOW, max_tokens: 256 };
    let answer = rag.generate(&run, &actx);

    assert!(answer.no_answer_reason.is_none(), "expected a grounded answer, got {:?}", answer.no_answer_reason);
    assert!(!answer.citations.is_empty(), "a grounded answer must cite at least one source");
    assert!(answer.invariant_holds());
    assert_eq!(answer.llm_provider, "mock-llm");
    assert_eq!(answer.prompt_template_version, "v1.0.0"); // AC-4
    // Every citation refers to a chunk that was actually retrieved (AC-6: no invented citations).
    let retrieved: Vec<&str> = run.results.iter().map(|r| r.chunk_id.as_str()).collect();
    for c in &answer.citations {
        assert!(retrieved.contains(&c.chunk_id.as_str()), "citation {} not in the run", c.chunk_id);
    }

    // R9/AC-4: the answer log is appended as JSON with the prompt version recorded.
    let log = paths.base.join("answers.jsonl").to_string_lossy().to_string();
    append_answer_log(&log, &answer).unwrap();
    let line = std::fs::read_to_string(&log).unwrap();
    let logged: serde_json::Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
    assert_eq!(logged["promptTemplateVersion"], "v1.0.0");
    assert!(!logged["citations"].as_array().unwrap().is_empty());
}
