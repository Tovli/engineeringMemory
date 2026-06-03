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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::ingestion::domain::Chunk;
    use crate::ingestion::infra::ruvector_store::RuVectorStoreAdapter;
    use crate::ingestion::ports::{ChunkWithEmbedding, VectorStorePort};

    const DIM: usize = 8;
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn paths() -> (String, String) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-rvsearch-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        (
            base.join("vectors.redb").to_string_lossy().to_string(),
            base.join("chunkmap.redb").to_string_lossy().to_string(),
        )
    }

    fn chunk(id: &str, doc: &str, source: &str, heading: Vec<String>) -> Chunk {
        let mut metadata = BTreeMap::new();
        // The ingestion adapter writes source_path into chunk metadata; the search adapter
        // reads it back via the `source_path` key.
        metadata.insert("source_path".to_string(), source.to_string());
        Chunk {
            id: id.into(),
            document_id: doc.into(),
            chunk_index: 0,
            heading_path: heading,
            content: format!("content {id}"),
            preview: format!("preview {id}"),
            content_hash: "hash".into(),
            token_count: 3,
            metadata,
        }
    }

    fn vec_of(seed: u8) -> Vec<f32> {
        (0..DIM).map(|i| ((seed as usize + i) % 7 + 1) as f32 / 8.0).collect()
    }

    #[test]
    fn parses_heading_path_source_and_core_fields() {
        let (vectors, chunkmap) = paths();
        {
            let store = RuVectorStoreAdapter::open(&vectors, &chunkmap, DIM).unwrap();
            let c = chunk("c1", "d1", "docs/x.md", vec!["Top".into(), "Sub".into()]);
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c, vector: vec_of(1) }]).unwrap();
        }
        let search = RuVectorSearchAdapter::open(&vectors, DIM).unwrap();
        let hits = search.vector_search(&vec_of(1), 5).unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!(h.chunk_id, "c1");
        assert_eq!(h.document_id, "d1");
        assert_eq!(h.source_path, "docs/x.md");
        assert_eq!(h.preview, "preview c1");
        // "Top > Sub" round-trips through the join/split back into a Vec.
        assert_eq!(h.heading_path, vec!["Top".to_string(), "Sub".to_string()]);
    }

    #[test]
    fn empty_heading_path_round_trips_to_an_empty_vec() {
        // join of [] is "" → split(" > ").filter(non-empty) must yield no segments, not [""].
        let (vectors, chunkmap) = paths();
        {
            let store = RuVectorStoreAdapter::open(&vectors, &chunkmap, DIM).unwrap();
            let c = chunk("c1", "d1", "docs/x.md", vec![]);
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c, vector: vec_of(2) }]).unwrap();
        }
        let search = RuVectorSearchAdapter::open(&vectors, DIM).unwrap();
        let hits = search.vector_search(&vec_of(2), 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].heading_path.is_empty(), "got {:?}", hits[0].heading_path);
    }
}
