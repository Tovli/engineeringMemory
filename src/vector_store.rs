//! VectorStore seam — isolates all RuVector calls behind a trait (PRD §15 Risk-1).
//!
//! `main.rs` depends only on this trait, never on `ruvector-core` directly, so the
//! future RuVector-Postgres / server backend can implement the same interface without
//! touching the rest of the application.

use std::collections::HashMap;

use ruvector_core::types::{DbOptions, HnswConfig, QuantizationConfig};
use ruvector_core::{DistanceMetric, SearchQuery, VectorDB, VectorEntry};
use serde_json::Value;

/// Bounded HNSW capacity shared by every `VectorDB` we open (M0 store + ingestion + retrieval).
/// `HnswConfig::default()` sets `max_elements = 10_000_000`, which pre-allocates several GB per
/// index instance — one fits, but parallel instances (e.g. concurrent tests) OOM. 100k is ample
/// headroom over the PRD's 5,000-chunk target (§9.1) at a fraction of the memory.
pub const MAX_INDEX_ELEMENTS: usize = 100_000;

/// The HNSW config used everywhere: defaults except a bounded `max_elements`.
pub fn default_hnsw_config() -> HnswConfig {
    HnswConfig { max_elements: MAX_INDEX_ELEMENTS, ..Default::default() }
}

/// A document to index.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: String,
    pub vector: Vec<f32>,
    pub title: String,
    pub topic: String,
    pub source: String,
}

/// A search hit, enriched with document metadata.
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: String,
    /// Distance score — lower is closer for distance metrics (Cosine here).
    pub score: f32,
    pub title: String,
    pub topic: String,
    pub source: String,
}

/// Storage-agnostic vector store. The future Postgres/server backend implements this.
pub trait VectorStore {
    fn upsert(&self, docs: &[Doc]) -> anyhow::Result<usize>;
    fn query(&self, vector: Vec<f32>, k: usize) -> anyhow::Result<Vec<Hit>>;
    fn count(&self) -> anyhow::Result<usize>;
}

/// RuVector-backed implementation using the embedded `ruvector-core` crate.
pub struct RuVectorStore {
    db: VectorDB,
}

impl RuVectorStore {
    /// Open (or create) an embedded RuVector store at `path` with Cosine distance
    /// and no quantization (exact distances for the spike).
    pub fn open(path: &str, dimensions: usize) -> anyhow::Result<Self> {
        let options = DbOptions {
            dimensions,
            distance_metric: DistanceMetric::Cosine,
            storage_path: path.to_string(),
            hnsw_config: Some(default_hnsw_config()),
            quantization: Some(QuantizationConfig::None),
        };
        let db = VectorDB::new(options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(Self { db })
    }
}

fn metadata_of(doc: &Doc) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert("title".to_string(), Value::String(doc.title.clone()));
    m.insert("topic".to_string(), Value::String(doc.topic.clone()));
    m.insert("source".to_string(), Value::String(doc.source.clone()));
    m
}

impl VectorStore for RuVectorStore {
    fn upsert(&self, docs: &[Doc]) -> anyhow::Result<usize> {
        let entries: Vec<VectorEntry> = docs
            .iter()
            .map(|d| VectorEntry {
                id: Some(d.id.clone()),
                vector: d.vector.clone(),
                metadata: Some(metadata_of(d)),
            })
            .collect();
        let ids = self
            .db
            .insert_batch(&entries)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(ids.len())
    }

    fn query(&self, vector: Vec<f32>, k: usize) -> anyhow::Result<Vec<Hit>> {
        let q = SearchQuery {
            vector,
            k,
            filter: None,
            ef_search: None,
        };
        let results = self.db.search(q).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let hits = results
            .into_iter()
            .map(|r| {
                let get = |key: &str| {
                    r.metadata
                        .as_ref()
                        .and_then(|m| m.get(key))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                Hit {
                    id: r.id,
                    score: r.score,
                    title: get("title"),
                    topic: get("topic"),
                    source: get("source"),
                }
            })
            .collect();
        Ok(hits)
    }

    fn count(&self) -> anyhow::Result<usize> {
        self.db.len().map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}
