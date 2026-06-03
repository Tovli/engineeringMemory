//! RuVectorSearchAdapter — read-only VectorSearchPort over ruvector-core's embedded VectorDB.
//! Reads the chunk metadata the M1 ingestion adapter wrote (document_id, source_path, preview,
//! heading_path joined with " > "). No native metadata filter is applied — filtering is done in
//! the application layer (ADR-0002). Distances are raw cosine distance (ADR-0003).

use std::collections::BTreeMap;

use ruvector_core::types::{DbOptions, QuantizationConfig};
use ruvector_core::{DistanceMetric, SearchQuery, VectorDB};
use serde_json::Value;

use crate::retrieval::ports::{RawSearchResult, VectorSearchPort};

pub struct RuVectorSearchAdapter {
    db: VectorDB,
}

impl RuVectorSearchAdapter {
    /// Open the existing index at `vector_path`. `dimension` must match the index it was built
    /// with (the caller derives it from the indexed model — see the CLI wiring).
    pub fn open(vector_path: &str, dimension: usize) -> anyhow::Result<Self> {
        let options = DbOptions {
            dimensions: dimension,
            distance_metric: DistanceMetric::Cosine,
            storage_path: vector_path.to_string(),
            hnsw_config: Some(crate::vector_store::default_hnsw_config()),
            quantization: Some(QuantizationConfig::None),
        };
        let db = VectorDB::new(options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(Self { db })
    }
}

fn str_of(md: &std::collections::HashMap<String, Value>, key: &str) -> String {
    md.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

impl VectorSearchPort for RuVectorSearchAdapter {
    fn vector_search(&self, query_vec: &[f32], k: usize) -> anyhow::Result<Vec<RawSearchResult>> {
        let q = SearchQuery { vector: query_vec.to_vec(), k, filter: None, ef_search: None };
        let results = self.db.search(q).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|r| {
                let md = r.metadata.unwrap_or_default();
                let heading = str_of(&md, "heading_path");
                let heading_path = heading
                    .split(" > ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                // Pass through all string-valued metadata for explain/output.
                let metadata: BTreeMap<String, String> = md
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();
                RawSearchResult {
                    chunk_id: r.id,
                    document_id: str_of(&md, "document_id"),
                    source_path: str_of(&md, "source_path"),
                    distance: r.score,
                    preview: str_of(&md, "preview"),
                    heading_path,
                    metadata,
                }
            })
            .collect())
    }
}
