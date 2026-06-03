//! tovli CLI — thin shell over the `tovli` library (PRD §9.4). No engine logic here.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use tovli::ingestion::chunking::ChunkingService;
use tovli::ingestion::domain::{ChunkingConfig, IngestionSummary};
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::infra::parsers::default_parsers;
use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
use tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
use tovli::ingestion::orchestrator::{IngestOptions, IngestionOrchestrator};
use tovli::ingestion::ports::Embedder;
use tovli::vector_store::{Doc, RuVectorStore, VectorStore};

#[derive(Parser)]
#[command(name = "tovli", about = "RuVector-based technical memory assistant", version)]
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

const STORE_DIR: &str = ".tovli";

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Spike => run_spike(),
        Command::Ingest(args) => run_ingest(args),
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
