//! tovli CLI — thin shell over the `tovli` library (PRD §9.4). No engine logic here.

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use tovli::ingestion::chunking::ChunkingService;
use tovli::ingestion::domain::{ChunkingConfig, IngestionSummary};
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::infra::parsers::default_parsers;
use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
use tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
use tovli::ingestion::model_guard::ensure_index_model_compatible;
use tovli::ingestion::orchestrator::{IngestOptions, IngestionOrchestrator};
use tovli::ingestion::ports::Embedder;
use tovli::retrieval::application::SearchService;
use tovli::retrieval::domain::{MetadataFilter, Query, RetrievalRun, RunReason, SearchMode};
use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
use tovli::retrieval::ports::DocumentLookupPort;
use tovli::evaluation::application::EvaluationService;
use tovli::evaluation::domain::{EvalRun, EvalRunConfig, EvalRunStatus, ThresholdConfig};
use tovli::evaluation::infra::dataset_loader::load_dataset;
use tovli::evaluation::infra::report_writer::write_report;
use tovli::evaluation::infra::retrieval_search_adapter::RetrievalSearchAdapter;
use tovli::vector_store::{Doc, RuVectorStore, VectorStore};

#[derive(Parser)]
#[command(name = "tovli", about = "RuVector-based technical memory assistant", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Directory holding the index files (vectors.redb, chunkmap.redb, documents.redb).
    /// Point this at a scratch dir to isolate experiments or CI from your working index.
    #[arg(long, global = true, env = "TOVLI_STORE", default_value = ".tovli")]
    store: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Run the M0 RuVector spike (embedded similarity-search demo).
    Spike,
    /// Ingest a folder of documents into the vector store.
    Ingest(IngestArgs),
    /// Search the indexed chunks by vector similarity.
    Search(SearchArgs),
    /// Evaluate retrieval quality against a ground-truth dataset.
    Eval(EvalArgs),
}

#[derive(Args)]
struct IngestArgs {
    /// Folder to ingest (scanned recursively).
    path: PathBuf,
    /// Report what would be ingested without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Re-ingest even unchanged files.
    #[arg(long)]
    force: bool,
    /// Tag every ingested document with this project name.
    #[arg(long)]
    project: Option<String>,
    /// Add a tag (repeatable).
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Use the deterministic mock embedder instead of local ONNX.
    #[arg(long)]
    mock: bool,
}

#[derive(Args)]
struct SearchArgs {
    /// The natural-language question.
    query: String,
    /// Maximum number of results to return.
    #[arg(long = "top-k", default_value_t = 8)]
    top_k: usize,
    /// Search mode. Only `vector` is available in Milestone 2 (keyword/hybrid arrive in M5).
    #[arg(long, default_value = "vector")]
    mode: String,
    /// Only return chunks from documents in this project.
    #[arg(long)]
    project: Option<String>,
    /// Only return chunks from documents carrying this tag (repeatable; all must match).
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Only return chunks from this exact source file.
    #[arg(long)]
    source: Option<String>,
    /// Show ranking/eligibility details for debugging retrieval.
    #[arg(long)]
    explain: bool,
    /// Use the deterministic mock embedder instead of local ONNX.
    #[arg(long)]
    mock: bool,
}

#[derive(Args)]
struct EvalArgs {
    /// Path to the evaluation questions JSON.
    path: PathBuf,
    /// Search mode. Only `vector` is available until Milestone 5.
    #[arg(long, default_value = "vector")]
    mode: String,
    /// Retrieval depth per question (at least 5 is used internally so Hit@5/MRR are computable).
    #[arg(long = "top-k", default_value_t = 5)]
    top_k: usize,
    /// Exit non-zero when Hit@3 falls below this fraction (CI regression gate).
    #[arg(long = "fail-below-hit-at-3")]
    fail_below_hit_at_3: Option<f64>,
    /// Where to write the JSON report.
    #[arg(long, default_value = "./eval/report.json")]
    output: String,
    /// Use the deterministic mock embedder instead of local ONNX.
    #[arg(long)]
    mock: bool,
}

/// The three redb files that make up an index, resolved under the `--store` directory.
struct StorePaths {
    vectors: String,
    chunkmap: String,
    documents: String,
}

fn store_paths(store: &Path) -> StorePaths {
    let p = |name: &str| store.join(name).to_string_lossy().to_string();
    StorePaths {
        vectors: p("vectors.redb"),
        chunkmap: p("chunkmap.redb"),
        documents: p("documents.redb"),
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Spike => run_spike(),
        Command::Ingest(args) => run_ingest(args, &cli.store),
        Command::Search(args) => run_search(args, &cli.store),
        Command::Eval(args) => run_eval(args, &cli.store),
    }
}

#[cfg(feature = "onnx")]
fn build_embedder(mock: bool) -> anyhow::Result<Box<dyn Embedder>> {
    if mock {
        Ok(Box::new(MockEmbedder::new(384)))
    } else {
        use tovli::ingestion::infra::onnx_embedder::OnnxEmbedder;
        Ok(Box::new(OnnxEmbedder::open_minilm()?))
    }
}

#[cfg(not(feature = "onnx"))]
fn build_embedder(_mock: bool) -> anyhow::Result<Box<dyn Embedder>> {
    // Built without the `onnx` feature → only the mock embedder is available.
    Ok(Box::new(MockEmbedder::new(384)))
}

fn run_ingest(args: IngestArgs, store_dir: &Path) -> anyhow::Result<()> {
    let paths = store_paths(store_dir);

    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    // Guard: refuse to mix embedding models in one index (write-side mirror of the retrieval
    // R6/AC-7 guard). Read the existing index model without creating the file, and drop the
    // read handle (inner scope) before opening the write handles below — redb locks per file.
    {
        let existing = RedbDocumentLookup::open(&paths.documents)?.indexed_model_version()?;
        ensure_index_model_compatible(existing.as_ref(), &model, args.force)
            .map_err(|e| anyhow::anyhow!(e))?;
    }

    std::fs::create_dir_all(store_dir)?;
    let dim = model.dimension;
    println!("embedder: {} (dim {dim})", model.name);

    let store = RuVectorStoreAdapter::open(&paths.vectors, &paths.chunkmap, dim)?;
    let docs = RedbDocumentRepository::open(&paths.documents)?;
    let parsers = default_parsers();
    let config = ChunkingConfig::default();
    config.validate().map_err(|e| anyhow::anyhow!(e))?;

    let orchestrator = IngestionOrchestrator {
        parsers: &parsers,
        embedder: embedder.as_ref(),
        store: &store,
        docs: &docs,
        chunking: ChunkingService::new(config),
    };

    let opts = IngestOptions {
        dry_run: args.dry_run,
        force: args.force,
        project: args.project,
        tags: args.tags,
    };
    let now = chrono::Utc::now().to_rfc3339();
    let summary = orchestrator.ingest(&args.path, &opts, &now)?;
    print_summary(&summary, &config);
    Ok(())
}

fn print_summary(s: &IngestionSummary, config: &ChunkingConfig) {
    println!("\n== ingestion summary{} ==", if s.dry_run { " (dry-run)" } else { "" });
    println!(
        "chunking: target={} max={} overlap={} tokens",
        config.target_tokens, config.max_tokens, config.overlap_tokens
    );
    println!("files scanned   : {}", s.files_scanned);
    println!("files ingested  : {}", s.files_ingested);
    println!("files unchanged : {}", s.files_unchanged);
    println!("files empty     : {}", s.files_empty);
    println!("files skipped   : {}", s.files_skipped);
    println!("files errored   : {}", s.files_errored);
    println!("files deleted   : {}", s.files_deleted);
    println!("chunks created  : {}", s.chunks_created);
    if !s.skipped.is_empty() {
        println!("skipped:");
        for (p, r) in &s.skipped {
            println!("  - {p}: {r}");
        }
    }
    if !s.errors.is_empty() {
        println!("errors:");
        for (p, r) in &s.errors {
            println!("  - {p}: {r}");
        }
    }
}

// ---------------------------------------------------------------------------
// M2 search (Retrieval context). CLI stays thin — all logic lives in SearchService.
// ---------------------------------------------------------------------------

fn run_search(args: SearchArgs, store_dir: &Path) -> anyhow::Result<()> {
    if args.mode != "vector" {
        eprintln!("mode '{}' is available in Milestone 5; use --mode vector", args.mode);
        std::process::exit(2);
    }

    let paths = store_paths(store_dir);
    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    let query = Query {
        text: args.query,
        mode: SearchMode::Vector,
        filters: MetadataFilter { project: args.project, tags: args.tags, source: args.source },
        top_k: args.top_k,
        embedding_model: model,
    };

    let lookup = RedbDocumentLookup::open(&paths.documents)?;

    // Size the vector store by the indexed model so we never open it with a wrong dimension.
    // No active document ⇒ empty index (AC-8) — report and exit 0 without opening the store.
    let Some(indexed) = lookup.indexed_model_version()? else {
        print_header(&query);
        println!("index is empty — run `tovli ingest <folder>` first");
        return Ok(());
    };
    let store = RuVectorSearchAdapter::open(&paths.vectors, indexed.dimension)?;

    let svc = SearchService { embedder: embedder.as_ref(), store: &store, lookup: &lookup };
    let run_id = format!("rrun_{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    let run = svc.search(&query, args.explain, &run_id, &now)?; // mismatch (AC-7) surfaces here
    print_results(&run);
    Ok(())
}

fn render_filters(f: &MetadataFilter) -> String {
    if f.is_empty() {
        return "(none)".to_string();
    }
    let mut parts = Vec::new();
    if let Some(p) = &f.project {
        parts.push(format!("project={p}"));
    }
    for t in &f.tags {
        parts.push(format!("tag={t}"));
    }
    if let Some(s) = &f.source {
        parts.push(format!("source={s}"));
    }
    parts.join("  ")
}

fn print_header(query: &Query) {
    println!("query  : \"{}\"   mode={}  top-k={}", query.text, query.mode, query.top_k);
    println!("filters: {}", render_filters(&query.filters));
}

fn print_results(run: &RetrievalRun) {
    print_header(&run.query);
    if run.reason == RunReason::IndexEmpty {
        println!("index is empty — run `tovli ingest <folder>` first");
        return;
    }
    if run.results.is_empty() {
        let suffix = if run.query.filters.is_empty() { "" } else { " for these filters" };
        println!("no results{suffix}");
        return;
    }
    for r in &run.results {
        println!("#{:<2} score={:.4}  {}", r.rank, r.score, r.source_path);
        if !r.heading_path.is_empty() {
            println!("      {}", r.heading_path.join(" > "));
        }
        let first_line = r.preview.lines().next().unwrap_or("");
        println!("      {first_line}   [{}]", r.chunk_id);
    }
    println!("\nlatency: {} ms   below-threshold: {}", run.latency_ms, run.below_threshold_count);
    if let Some(ex) = &run.explain {
        println!("\n== explain ==");
        println!("provider     : {} (dim {})", ex.query_embedding_provider, ex.query_embedding_dimension);
        println!("search mode  : {}", ex.search_mode);
        println!("ranking      : {}", ex.ranking_method);
        println!("filters      : {}", render_filters(&ex.filters_applied));
        for d in &ex.result_details {
            println!(
                "  #{:<2} chunk={} vector={:.4} fused={:.4} :: {}",
                d.rank,
                d.chunk_id,
                d.vector_score.unwrap_or(0.0),
                d.fused_score,
                d.eligibility_reason
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M3 eval (Evaluation context). CLI loads dataset, delegates to EvaluationService, writes report.
// ---------------------------------------------------------------------------

fn run_eval(args: EvalArgs, store_dir: &Path) -> anyhow::Result<()> {
    if args.mode != "vector" {
        eprintln!("mode '{}' is available in Milestone 5; use --mode vector", args.mode);
        std::process::exit(2);
    }
    let paths = store_paths(store_dir);
    let dataset_path = args.path.to_string_lossy().to_string();
    let questions = load_dataset(&dataset_path)?;

    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    let lookup = RedbDocumentLookup::open(&paths.documents)?;
    let dim = lookup.indexed_model_version()?.map(|m| m.dimension).unwrap_or(model.dimension);
    let store = RuVectorSearchAdapter::open(&paths.vectors, dim)?;

    let search = SearchService { embedder: embedder.as_ref(), store: &store, lookup: &lookup };
    let adapter = RetrievalSearchAdapter { inner: search };
    let evaluator = EvaluationService { search: &adapter };

    let config = EvalRunConfig {
        mode: SearchMode::Vector,
        top_k: args.top_k,
        threshold: ThresholdConfig { min_hit_at_3: args.fail_below_hit_at_3 },
        embedding_model: model,
    };
    let run_id = format!("ev_{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    let run = evaluator.run(&questions, &config, &dataset_path, &run_id, &now);

    print_eval(&run);
    write_report(&args.output, &run)?;
    println!("report written to {}", args.output);

    match run.status {
        EvalRunStatus::Failed => {
            eprintln!("evaluation failed: {}", run.error.unwrap_or_default());
            std::process::exit(1);
        }
        EvalRunStatus::ThresholdFailed => {
            eprintln!(
                "Hit@3 {:.2} is below the configured threshold {:.2}",
                run.metrics.hit_at_3,
                config.threshold.min_hit_at_3.unwrap_or(0.0)
            );
            std::process::exit(1);
        }
        EvalRunStatus::Completed => Ok(()),
    }
}

fn print_eval(run: &EvalRun) {
    let m = &run.metrics;
    println!("== evaluation ==");
    println!("dataset        : {}", run.dataset_path);
    println!("mode           : {}  top-k={}", run.search_mode, run.top_k);
    println!("model          : {} (dim {})", run.embedding_model.name, run.embedding_model.dimension);
    println!("questions      : {}", m.question_count);
    println!("Hit@1/3/5      : {:.2} / {:.2} / {:.2}", m.hit_at_1, m.hit_at_3, m.hit_at_5);
    println!("MRR            : {:.3}", m.mrr);
    println!("avg latency    : {:.1} ms", m.avg_latency_ms);
    println!("empty results  : {}", m.empty_result_count);
    println!("below threshold: {}", m.below_threshold_count);
}

// ---------------------------------------------------------------------------
// M0 spike (preserved): deterministic 4-dim corpus, k-NN, self-check.
// ---------------------------------------------------------------------------

fn sample_docs() -> Vec<Doc> {
    let mk = |id: &str, v: [f32; 4], title: &str, topic: &str, source: &str| Doc {
        id: id.to_string(),
        vector: v.to_vec(),
        title: title.to_string(),
        topic: topic.to_string(),
        source: source.to_string(),
    };
    vec![
        mk("doc-arch-1", [1.0, 0.0, 0.0, 0.1], "Architecture layering rules", "architecture", "docs/architecture.md"),
        mk("doc-arch-2", [0.9, 0.1, 0.0, 0.0], "Component vs deployment boundaries", "architecture", "docs/boundaries.md"),
        mk("doc-deploy-1", [0.0, 1.0, 0.0, 0.1], "Azure Function zipDeploy 403", "deployment", "docs/deploy-azure.md"),
        mk("doc-deploy-2", [0.1, 0.9, 0.0, 0.0], "GitHub Actions npm ci failure", "deployment", "docs/deploy-ci.md"),
        mk("doc-auth-1", [0.0, 0.0, 1.0, 0.1], "Firebase auth migration", "auth", "docs/auth-firebase.md"),
        mk("doc-auth-2", [0.0, 0.1, 0.9, 0.0], "Token refresh conventions", "auth", "docs/auth-tokens.md"),
    ]
}

fn run_spike() -> anyhow::Result<()> {
    let db_path = std::env::temp_dir().join("tovli-m0-spike.redb");
    let _ = std::fs::remove_file(&db_path);
    let db_path_str = db_path.to_string_lossy().to_string();

    println!("== tovli M0 RuVector spike ==");
    println!("engine : ruvector-core (embedded, no Docker)");
    println!("store  : {db_path_str}\n");

    let store = RuVectorStore::open(&db_path_str, 4)?;
    let docs = sample_docs();
    let ingested = store.upsert(&docs)?;
    let count = store.count()?;
    println!("ingested {ingested} docs, store count = {count}\n");

    let query = vec![0.95_f32, 0.05, 0.0, 0.0];
    let started = std::time::Instant::now();
    let hits = store.query(query.clone(), 3)?;
    let latency = started.elapsed();

    println!("query  : {query:?}  (k=3, cosine distance — lower = closer)");
    for (i, h) in hits.iter().enumerate() {
        println!(
            "  #{rank}  {id:<13} dist={score:.4}  topic={topic:<13} {title}  [{source}]",
            rank = i + 1,
            id = h.id,
            score = h.score,
            topic = h.topic,
            title = h.title,
            source = h.source,
        );
    }
    println!("latency: {latency:?}\n");

    let scores_sorted = hits.windows(2).all(|w| w[0].score <= w[1].score + 1e-6);
    let checks: Vec<(&str, bool)> = vec![
        ("count == 6", count == 6),
        ("top-1 is doc-arch-2", hits.first().map(|h| h.id.as_str()) == Some("doc-arch-2")),
        ("top-2 is doc-arch-1", hits.get(1).map(|h| h.id.as_str()) == Some("doc-arch-1")),
        ("top-2 both architecture", hits.iter().take(2).all(|h| h.topic == "architecture")),
        ("scores non-decreasing", scores_sorted),
    ];

    println!("checks:");
    let mut all_ok = true;
    for (name, ok) in &checks {
        println!("  [{}] {name}", if *ok { "PASS" } else { "FAIL" });
        all_ok &= ok;
    }

    if all_ok {
        println!("\nRESULT: PASS — RuVector embedded retrieval works locally (M0 acceptance met).");
        Ok(())
    } else {
        eprintln!("\nRESULT: FAIL — see failing checks above.");
        std::process::exit(1);
    }
}
