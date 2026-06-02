//! tovli — M0 RuVector spike.
//!
//! Proves RuVector can be embedded and queried from Rust: ingest vectors with metadata,
//! run a k-NN similarity search, and verify the nearest neighbours match docs/spike/M0-spec.md.
//! All RuVector access goes through the `VectorStore` seam — no engine logic lives here.

mod vector_store;

use vector_store::{Doc, RuVectorStore, VectorStore};

/// Deterministic 4-dim sample corpus: 3 topic clusters, 2 docs each (see M0-spec.md).
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

fn main() -> anyhow::Result<()> {
    // Fresh, isolated DB file each run for determinism.
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

    // "Architecture-flavoured" query (see spec).
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

    // ---- Automated success criteria (M0-spec.md) ----
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
