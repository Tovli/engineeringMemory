//! tovli CLI — thin shell over the `tovli` library (PRD §9.4). No engine logic here.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use tovli::answer_generation::application::rag_service::{AnswerContext, RagAnswerService};
use tovli::answer_generation::domain::answer::{Answer, NoAnswerReason};
use tovli::answer_generation::infra::answer_log_writer::append_answer_log;
use tovli::answer_generation::infra::mock_llm::MockLlm;
use tovli::evaluation::application::EvaluationService;
use tovli::evaluation::domain::{EvalRun, EvalRunConfig, EvalRunStatus, ThresholdConfig};
use tovli::evaluation::infra::dataset_loader::load_dataset;
use tovli::evaluation::infra::report_writer::write_report;
use tovli::evaluation::infra::retrieval_search_adapter::RetrievalSearchAdapter;
use tovli::feedback::application::{
    next_feedback_ids, next_query_run_ids, FeedbackReportService, FeedbackService,
    RecordFeedbackInput,
};
use tovli::feedback::domain::{FeedbackRating, FeedbackReport, RetrievalRunEvidence};
use tovli::feedback::infra::{export_feedback_json, JsonlRetrievalRunLog, RedbFeedbackRepository};
use tovli::ingestion::chunking::ChunkingService;
use tovli::ingestion::domain::{ChunkingConfig, IngestionSummary};
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::infra::parsers::default_parsers;
use tovli::ingestion::infra::redb_keyword_index::RedbKeywordIndex;
use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
use tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
use tovli::ingestion::orchestrator::{IngestOptions, IngestionOrchestrator};
use tovli::ingestion::ports::Embedder;
use tovli::retrieval::application::SearchService;
use tovli::retrieval::domain::{MetadataFilter, Query, RetrievalRun, RunReason, SearchMode};
use tovli::retrieval::infra::redb_keyword_search::RedbKeywordSearch;
use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
use tovli::retrieval::ports::DocumentLookupPort;
use tovli::vector_store::{Doc, RuVectorStore, VectorStore};

#[derive(Parser)]
#[command(
    name = "tovli",
    about = "RuVector-based technical memory assistant",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
    /// Ask a question and get a cited answer generated from retrieved chunks (RAG).
    Ask(AskArgs),
    /// Record useful/not-useful feedback for chunks returned by a prior search/ask run.
    Feedback(FeedbackArgs),
    /// Report problematic retrieval patterns from stored feedback.
    FeedbackReport(FeedbackReportArgs),
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
    /// Search mode.
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
    /// Search mode.
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

#[derive(Args)]
struct AskArgs {
    /// The natural-language question.
    query: String,
    /// Maximum number of chunks to retrieve as context.
    #[arg(long = "top-k", default_value_t = 8)]
    top_k: usize,
    /// Search mode.
    #[arg(long, default_value = "vector")]
    mode: String,
    /// Run retrieval only — print the context and skip answer generation (no LLM call).
    #[arg(long = "no-llm")]
    no_llm: bool,
    /// Print the retrieved chunks (rank, score, source, preview) alongside the answer.
    #[arg(long = "show-context")]
    show_context: bool,
    /// Use the deterministic mock embedder instead of local ONNX.
    #[arg(long)]
    mock: bool,
}

#[derive(Args)]
struct FeedbackArgs {
    /// Query id printed by `search` or `ask`.
    #[arg(long = "query-id")]
    query_id: String,
    /// Retrieval run id printed by `search` or `ask`; defaults to the latest run for query-id.
    #[arg(long = "run-id", alias = "retrieval-run-id")]
    run_id: Option<String>,
    /// Mark this displayed chunk as useful (repeatable).
    #[arg(long = "good")]
    good: Vec<String>,
    /// Mark this displayed chunk as not useful (repeatable).
    #[arg(long = "bad")]
    bad: Vec<String>,
    /// Optional note stored with every feedback item in this command.
    #[arg(long)]
    note: Option<String>,
}

#[derive(Args)]
struct FeedbackReportArgs {
    /// Export the raw append-only feedback log as JSON.
    #[arg(long)]
    export: Option<String>,
}

const STORE_DIR: &str = ".tovli";
const KEYWORD_INDEX_FILE: &str = "keyword.redb";
const RETRIEVAL_RUN_LOG_FILE: &str = "retrieval-runs.jsonl";
const FEEDBACK_FILE: &str = "feedback.redb";

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Spike => run_spike(),
        Command::Ingest(args) => run_ingest(args),
        Command::Search(args) => run_search(args),
        Command::Eval(args) => run_eval(args),
        Command::Ask(args) => run_ask(args),
        Command::Feedback(args) => run_feedback(args),
        Command::FeedbackReport(args) => run_feedback_report(args),
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

fn parse_mode_or_exit(raw: &str) -> SearchMode {
    match raw.parse::<SearchMode>() {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}

fn run_ingest(args: IngestArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(STORE_DIR)?;

    let embedder = build_embedder(args.mock)?;
    let dim = embedder.model_version().dimension;
    println!("embedder: {} (dim {dim})", embedder.model_version().name);

    let store = RuVectorStoreAdapter::open(
        &format!("{STORE_DIR}/vectors.redb"),
        &format!("{STORE_DIR}/chunkmap.redb"),
        dim,
    )?;
    let docs = RedbDocumentRepository::open(&format!("{STORE_DIR}/documents.redb"))?;
    let keyword_index = RedbKeywordIndex::open(&format!("{STORE_DIR}/{KEYWORD_INDEX_FILE}"))?;
    let parsers = default_parsers();
    let config = ChunkingConfig::default();
    config.validate().map_err(|e| anyhow::anyhow!(e))?;

    let orchestrator = IngestionOrchestrator {
        parsers: &parsers,
        embedder: embedder.as_ref(),
        store: &store,
        keyword_index: Some(&keyword_index),
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
    println!(
        "\n== ingestion summary{} ==",
        if s.dry_run { " (dry-run)" } else { "" }
    );
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

fn run_search(args: SearchArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(STORE_DIR)?;
    let mode = parse_mode_or_exit(&args.mode);

    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    let query = Query {
        text: args.query,
        mode,
        filters: MetadataFilter {
            project: args.project,
            tags: args.tags,
            source: args.source,
        },
        top_k: args.top_k,
        embedding_model: model,
    };
    let ids = next_query_run_ids(chrono::Utc::now().timestamp_millis());
    let query_id = ids.query_id;
    let run_id = ids.retrieval_run_id;
    let now = chrono::Utc::now().to_rfc3339();

    let lookup = RedbDocumentLookup::open(&format!("{STORE_DIR}/documents.redb"))?;

    // Size the vector store by the indexed model so we never open it with a wrong dimension.
    // No active document ⇒ empty index (AC-8) — report and exit 0 without opening the store.
    let Some(indexed) = lookup.indexed_model_version()? else {
        let run = empty_retrieval_run(&query, &run_id, &now);
        append_run_evidence(&run, &query_id)?;
        println!("query-id: {query_id}   run-id: {run_id}");
        print_results(&run);
        return Ok(());
    };
    let store =
        RuVectorSearchAdapter::open(&format!("{STORE_DIR}/vectors.redb"), indexed.dimension)?;
    let keyword = RedbKeywordSearch::open(&format!("{STORE_DIR}/{KEYWORD_INDEX_FILE}"))?;

    let svc = SearchService {
        embedder: embedder.as_ref(),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };
    let run = svc.search(&query, args.explain, &run_id, &now)?; // mismatch (AC-7) surfaces here
    append_run_evidence(&run, &query_id)?;
    println!("query-id: {query_id}   run-id: {run_id}");
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
    println!(
        "query  : \"{}\"   mode={}  top-k={}",
        query.text, query.mode, query.top_k
    );
    println!("filters: {}", render_filters(&query.filters));
}

fn print_results(run: &RetrievalRun) {
    print_header(&run.query);
    if run.reason == RunReason::IndexEmpty {
        println!("index is empty — run `tovli ingest <folder>` first");
        return;
    }
    if run.results.is_empty() {
        let suffix = if run.query.filters.is_empty() {
            ""
        } else {
            " for these filters"
        };
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
    println!(
        "\nlatency: {} ms   below-threshold: {}",
        run.latency_ms, run.below_threshold_count
    );
    if let Some(ex) = &run.explain {
        println!("\n== explain ==");
        println!(
            "provider     : {} (dim {})",
            ex.query_embedding_provider, ex.query_embedding_dimension
        );
        println!("search mode  : {}", ex.search_mode);
        println!("ranking      : {}", ex.ranking_method);
        println!("filters      : {}", render_filters(&ex.filters_applied));
        for d in &ex.result_details {
            let vector = d
                .vector_score
                .map(|s| format!("{s:.4}"))
                .unwrap_or_else(|| "-".to_string());
            let keyword = d
                .keyword_score
                .map(|s| format!("{s:.4}"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  #{:<2} chunk={} vector={} keyword={} fused={:.4} :: {}",
                d.rank, d.chunk_id, vector, keyword, d.fused_score, d.eligibility_reason
            );
        }
    }
}

// ---------------------------------------------------------------------------
// M3 eval (Evaluation context). CLI loads dataset, delegates to EvaluationService, writes report.
// ---------------------------------------------------------------------------

fn run_eval(args: EvalArgs) -> anyhow::Result<()> {
    let mode = parse_mode_or_exit(&args.mode);
    let dataset_path = args.path.to_string_lossy().to_string();
    let questions = load_dataset(&dataset_path)?;

    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    let lookup = RedbDocumentLookup::open(&format!("{STORE_DIR}/documents.redb"))?;
    let dim = lookup
        .indexed_model_version()?
        .map(|m| m.dimension)
        .unwrap_or(model.dimension);
    let store = RuVectorSearchAdapter::open(&format!("{STORE_DIR}/vectors.redb"), dim)?;
    let keyword = RedbKeywordSearch::open(&format!("{STORE_DIR}/{KEYWORD_INDEX_FILE}"))?;

    let search = SearchService {
        embedder: embedder.as_ref(),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };
    let adapter = RetrievalSearchAdapter { inner: search };
    let evaluator = EvaluationService { search: &adapter };

    let config = EvalRunConfig {
        mode,
        top_k: args.top_k,
        threshold: ThresholdConfig {
            min_hit_at_3: args.fail_below_hit_at_3,
        },
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
    println!(
        "model          : {} (dim {})",
        run.embedding_model.name, run.embedding_model.dimension
    );
    println!("questions      : {}", m.question_count);
    println!(
        "Hit@1/3/5      : {:.2} / {:.2} / {:.2}",
        m.hit_at_1, m.hit_at_3, m.hit_at_5
    );
    println!("MRR            : {:.3}", m.mrr);
    println!("avg latency    : {:.1} ms", m.avg_latency_ms);
    println!("empty results  : {}", m.empty_result_count);
    println!("below threshold: {}", m.below_threshold_count);
}

// ---------------------------------------------------------------------------
// M4 ask (Answer Generation context). CLI retrieves (M2) then delegates generation to
// RagAnswerService; the domain enforces citations/no-answer. Thin handler (C5/R12).
// ---------------------------------------------------------------------------

fn run_ask(args: AskArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(STORE_DIR)?;
    let mode = parse_mode_or_exit(&args.mode);

    let embedder = build_embedder(args.mock)?;
    let model = embedder.model_version().clone();

    let query = Query {
        text: args.query,
        mode,
        filters: MetadataFilter::default(),
        top_k: args.top_k,
        embedding_model: model,
    };
    let ts = chrono::Utc::now().timestamp_millis();
    let ids = next_query_run_ids(ts);
    let query_id = ids.query_id;
    let run_id = ids.retrieval_run_id;
    let now = chrono::Utc::now().to_rfc3339();
    print_header(&query);

    let lookup = RedbDocumentLookup::open(&format!("{STORE_DIR}/documents.redb"))?;
    // Empty index (no active document) → nothing to ground an answer on (AC-2 territory) — exit 0.
    let Some(indexed) = lookup.indexed_model_version()? else {
        let run = empty_retrieval_run(&query, &run_id, &now);
        append_run_evidence(&run, &query_id)?;
        println!("query-id: {query_id}   run-id: {run_id}");
        println!("index is empty — run `tovli ingest <folder>` first");
        return Ok(());
    };
    let store =
        RuVectorSearchAdapter::open(&format!("{STORE_DIR}/vectors.redb"), indexed.dimension)?;
    let keyword = RedbKeywordSearch::open(&format!("{STORE_DIR}/{KEYWORD_INDEX_FILE}"))?;

    let svc = SearchService {
        embedder: embedder.as_ref(),
        store: &store,
        keyword: &keyword,
        lookup: &lookup,
    };
    let run = svc.search(&query, false, &run_id, &now)?; // model mismatch surfaces here
    append_run_evidence(&run, &query_id)?;
    println!("query-id: {query_id}   run-id: {run_id}");

    if args.show_context || args.no_llm {
        print_context(&run);
    }
    if args.no_llm {
        // Retrieval-only mode (AC-5): never construct the LLM provider.
        return Ok(());
    }

    let llm = MockLlm::default();
    let rag = RagAnswerService { llm: &llm };
    let answer_id = format!("ans_{ts}");
    let actx = AnswerContext {
        query_id: &query_id,
        answer_id: &answer_id,
        now: &now,
        max_tokens: 512,
    };
    let answer = rag.generate(&run, &actx);
    print_answer(&answer);
    append_answer_log(&format!("{STORE_DIR}/answers.jsonl"), &answer)?;

    // Exit codes (ADR-0008 / D-EXITCODE): a no-answer is a valid product response (exit 0); only a
    // provider failure is a non-zero infra error so scripts/CI can distinguish "no source" from "broken".
    if answer.no_answer_reason == Some(NoAnswerReason::LlmProviderError) {
        std::process::exit(3);
    }
    Ok(())
}

fn run_feedback(args: FeedbackArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(STORE_DIR)?;
    if args.good.is_empty() && args.bad.is_empty() {
        anyhow::bail!("provide at least one --good <chunk-id> or --bad <chunk-id>");
    }

    let runs = JsonlRetrievalRunLog::open(&format!("{STORE_DIR}/{RETRIEVAL_RUN_LOG_FILE}"));
    let run_id = match args.run_id {
        Some(id) => id,
        None => runs
            .find_latest_by_query_id(&args.query_id)?
            .map(|run| run.retrieval_run_id)
            .ok_or_else(|| {
                anyhow::anyhow!("no retrieval run found for query id '{}'", args.query_id)
            })?,
    };
    let repo = RedbFeedbackRepository::open(&format!("{STORE_DIR}/{FEEDBACK_FILE}"))?;
    let service = FeedbackService {
        feedback: &repo,
        runs: &runs,
    };

    let now = chrono::Utc::now().to_rfc3339();
    let seed = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() * 1_000_000);
    let feedback_ids = next_feedback_ids(seed, args.good.len() + args.bad.len());
    let inputs: Vec<RecordFeedbackInput> = args
        .good
        .iter()
        .map(|id| (FeedbackRating::Good, id))
        .chain(args.bad.iter().map(|id| (FeedbackRating::Bad, id)))
        .zip(feedback_ids)
        .map(|((rating, chunk_id), feedback_id)| RecordFeedbackInput {
            feedback_id,
            query_id: args.query_id.clone(),
            retrieval_run_id: run_id.clone(),
            chunk_id: chunk_id.clone(),
            rating,
            note: args.note.clone(),
            created_at: now.clone(),
        })
        .collect();
    let items = service.record_many(inputs)?;
    for item in items {
        println!(
            "recorded feedback {}: {} {} (query-id: {}   run-id: {})",
            item.id, item.rating, item.chunk_id, item.query_id, item.retrieval_run_id
        );
    }
    Ok(())
}

fn run_feedback_report(args: FeedbackReportArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(STORE_DIR)?;
    let repo = RedbFeedbackRepository::open(&format!("{STORE_DIR}/{FEEDBACK_FILE}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let report = FeedbackReportService { feedback: &repo }.generate(
        &format!("rpt_{}", chrono::Utc::now().timestamp_millis()),
        &now,
    )?;
    print_feedback_report(&report);
    if let Some(path) = args.export {
        export_feedback_json(&path, &repo)?;
        println!("exported feedback: {path}");
    }
    Ok(())
}

fn append_run_evidence(run: &RetrievalRun, query_id: &str) -> anyhow::Result<()> {
    let log = JsonlRetrievalRunLog::open(&format!("{STORE_DIR}/{RETRIEVAL_RUN_LOG_FILE}"));
    log.append(&RetrievalRunEvidence::from_run(run, query_id))
}

fn empty_retrieval_run(query: &Query, run_id: &str, now: &str) -> RetrievalRun {
    RetrievalRun {
        id: run_id.to_string(),
        query: query.clone(),
        results: vec![],
        search_mode: query.mode,
        top_k: query.top_k,
        latency_ms: 0,
        below_threshold_count: 0,
        reason: RunReason::IndexEmpty,
        explain: None,
        completed_at: now.to_string(),
    }
}

fn print_feedback_report(report: &FeedbackReport) {
    println!("== feedback report ==");
    println!("report id      : {}", report.id);
    println!("generated at   : {}", report.generated_at);
    println!("total feedback : {}", report.total_feedback);
    println!("observations    : {}", report.observations.len());
    for o in &report.observations {
        println!(
            "  - {} {} {} run={} rank={} score={:.4} mode={} {}",
            o.feedback_id,
            o.rating,
            o.chunk_id,
            o.retrieval_run_id,
            o.rank,
            o.score,
            o.search_mode,
            o.source_path
        );
    }
    println!("problem queries: {}", report.problematic_queries.len());
    for q in &report.problematic_queries {
        println!(
            "  - {} bad={} good={} bad-ratio={:.2} modes={}",
            q.query_id,
            q.bad_count,
            q.good_count,
            q.bad_ratio,
            q.search_modes
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        println!("    {}", q.question_text);
    }
    println!(
        "downvoted chunks: {}",
        report.frequently_downvoted_chunks.len()
    );
    for c in &report.frequently_downvoted_chunks {
        println!(
            "  - {} bad={} queries={} {}",
            c.chunk_id, c.bad_count, c.distinct_query_count, c.source_path
        );
    }
    println!(
        "no-good queries : {}",
        report.queries_with_no_good_result.len()
    );
    for q in &report.queries_with_no_good_result {
        println!(
            "  - {} feedback={} good={}",
            q.query_id, q.total_feedback, q.good_count
        );
        println!("    {}", q.question_text);
    }
    println!(
        "rechunking candidates: {}",
        report.candidates_for_rechunking.len()
    );
    for c in &report.candidates_for_rechunking {
        println!(
            "  - {} chunks={} {}",
            c.document_id, c.downvoted_chunk_count, c.source_path
        );
    }
}

fn print_context(run: &RetrievalRun) {
    println!("\n== retrieved context ==");
    if run.results.is_empty() {
        println!("(no chunks retrieved)");
        return;
    }
    for r in &run.results {
        println!(
            "#{:<2} score={:.4}  {}  [{}]",
            r.rank, r.score, r.source_path, r.chunk_id
        );
        let first_line = r.preview.lines().next().unwrap_or("");
        if !first_line.is_empty() {
            println!("      {first_line}");
        }
    }
}

fn print_answer(answer: &Answer) {
    println!("\n== answer ==");
    println!("{}", answer.answer_text);
    match &answer.no_answer_reason {
        Some(reason) => println!("\n(no answer: {reason:?})"),
        None => {
            println!("\nSources:");
            for (i, c) in answer.citations.iter().enumerate() {
                println!("{}. {}#{}", i + 1, c.source_path, c.chunk_id);
            }
        }
    }
    println!(
        "\nprompt: {}   provider: {}",
        answer.prompt_template_version, answer.llm_provider
    );
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
        mk(
            "doc-arch-1",
            [1.0, 0.0, 0.0, 0.1],
            "Architecture layering rules",
            "architecture",
            "docs/architecture.md",
        ),
        mk(
            "doc-arch-2",
            [0.9, 0.1, 0.0, 0.0],
            "Component vs deployment boundaries",
            "architecture",
            "docs/boundaries.md",
        ),
        mk(
            "doc-deploy-1",
            [0.0, 1.0, 0.0, 0.1],
            "Azure Function zipDeploy 403",
            "deployment",
            "docs/deploy-azure.md",
        ),
        mk(
            "doc-deploy-2",
            [0.1, 0.9, 0.0, 0.0],
            "GitHub Actions npm ci failure",
            "deployment",
            "docs/deploy-ci.md",
        ),
        mk(
            "doc-auth-1",
            [0.0, 0.0, 1.0, 0.1],
            "Firebase auth migration",
            "auth",
            "docs/auth-firebase.md",
        ),
        mk(
            "doc-auth-2",
            [0.0, 0.1, 0.9, 0.0],
            "Token refresh conventions",
            "auth",
            "docs/auth-tokens.md",
        ),
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
        (
            "top-1 is doc-arch-2",
            hits.first().map(|h| h.id.as_str()) == Some("doc-arch-2"),
        ),
        (
            "top-2 is doc-arch-1",
            hits.get(1).map(|h| h.id.as_str()) == Some("doc-arch-1"),
        ),
        (
            "top-2 both architecture",
            hits.iter().take(2).all(|h| h.topic == "architecture"),
        ),
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
