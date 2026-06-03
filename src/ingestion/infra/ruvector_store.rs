//! RuVectorStoreAdapter — VectorStorePort over ruvector-core's embedded VectorDB.
//! Persists a document_id → [chunk_id] map in a redb sidecar so a document's vectors
//! can be deleted on re-chunk / file removal (ruvector-core deletes by id).

use std::collections::HashMap;

use redb::{Database, ReadableTable, TableDefinition};
use ruvector_core::types::{DbOptions, QuantizationConfig};
use ruvector_core::{DistanceMetric, VectorDB, VectorEntry};
use serde_json::Value;

use crate::ingestion::domain::DocumentId;
use crate::ingestion::ports::{ChunkWithEmbedding, VectorStorePort};

const DOC_CHUNKS: TableDefinition<&str, &str> = TableDefinition::new("doc_chunks");

pub struct RuVectorStoreAdapter {
    db: VectorDB,
    map: Database,
    dim: usize,
}

impl RuVectorStoreAdapter {
    pub fn open(vector_path: &str, map_path: &str, dimension: usize) -> anyhow::Result<Self> {
        let options = DbOptions {
            dimensions: dimension,
            distance_metric: DistanceMetric::Cosine,
            storage_path: vector_path.to_string(),
            hnsw_config: Some(crate::vector_store::default_hnsw_config()),
            quantization: Some(QuantizationConfig::None),
        };
        let db = VectorDB::new(options).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let map = Database::create(map_path)?;
        Ok(Self { db, map, dim: dimension })
    }

    fn read_chunk_ids(&self, doc_id: &str) -> anyhow::Result<Vec<String>> {
        let rtxn = self.map.begin_read()?;
        let table = match rtxn.open_table(DOC_CHUNKS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        match table.get(doc_id)? {
            Some(g) => Ok(serde_json::from_str(g.value()).unwrap_or_default()),
            None => Ok(vec![]),
        }
    }
}

impl VectorStorePort for RuVectorStoreAdapter {
    fn upsert_chunks(&self, items: &[ChunkWithEmbedding]) -> anyhow::Result<()> {
        let mut per_doc: HashMap<String, Vec<String>> = HashMap::new();
        for it in items {
            if it.vector.len() != self.dim {
                anyhow::bail!("dim mismatch: vector {} != index {}", it.vector.len(), self.dim);
            }
            let mut md: HashMap<String, Value> = HashMap::new();
            for (k, v) in &it.chunk.metadata {
                md.insert(k.clone(), Value::String(v.clone()));
            }
            md.insert("document_id".into(), Value::String(it.chunk.document_id.clone()));
            md.insert("chunk_index".into(), Value::from(it.chunk.chunk_index));
            md.insert("heading_path".into(), Value::String(it.chunk.heading_path.join(" > ")));
            md.insert("content_hash".into(), Value::String(it.chunk.content_hash.clone()));
            md.insert("token_count".into(), Value::from(it.chunk.token_count));
            md.insert("preview".into(), Value::String(it.chunk.preview.clone()));

            self.db
                .insert(VectorEntry {
                    id: Some(it.chunk.id.clone()),
                    vector: it.vector.clone(),
                    metadata: Some(md),
                })
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            per_doc
                .entry(it.chunk.document_id.clone())
                .or_default()
                .push(it.chunk.id.clone());
        }

        let wtxn = self.map.begin_write()?;
        {
            let mut table = wtxn.open_table(DOC_CHUNKS)?;
            for (doc_id, mut ids) in per_doc {
                let existing: Vec<String> = match table.get(doc_id.as_str())? {
                    Some(g) => serde_json::from_str(g.value()).unwrap_or_default(),
                    None => vec![],
                };
                let mut all = existing;
                all.append(&mut ids);
                let json = serde_json::to_string(&all)?;
                table.insert(doc_id.as_str(), json.as_str())?;
            }
        }
        wtxn.commit()?;
        Ok(())
    }

    fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()> {
        for chunk_id in self.read_chunk_ids(id)? {
            self.db.delete(&chunk_id).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        let wtxn = self.map.begin_write()?;
        {
            let mut table = wtxn.open_table(DOC_CHUNKS)?;
            table.remove(id.as_str())?;
        }
        wtxn.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::ingestion::domain::Chunk;
    use crate::retrieval::infra::ruvector_search::RuVectorSearchAdapter;
    use crate::retrieval::ports::VectorSearchPort;

    const DIM: usize = 8;
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Paths {
        vectors: String,
        chunkmap: String,
    }
    fn paths() -> Paths {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-rvstore-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        Paths {
            vectors: base.join("vectors.redb").to_string_lossy().to_string(),
            chunkmap: base.join("chunkmap.redb").to_string_lossy().to_string(),
        }
    }

    fn chunk(id: &str, doc: &str, idx: u32) -> Chunk {
        Chunk {
            id: id.into(),
            document_id: doc.into(),
            chunk_index: idx,
            heading_path: vec!["H".into()],
            content: format!("content {id}"),
            preview: format!("preview {id}"),
            content_hash: "hash".into(),
            token_count: 3,
            metadata: BTreeMap::new(),
        }
    }

    /// Distinct, normalizable vectors so cosine search always returns the present rows.
    fn vec_of(seed: u8) -> Vec<f32> {
        (0..DIM).map(|i| ((seed as usize + i) % 7 + 1) as f32 / 8.0).collect()
    }

    #[test]
    fn upsert_then_search_round_trips_the_chunk() {
        let p = paths();
        {
            let store = RuVectorStoreAdapter::open(&p.vectors, &p.chunkmap, DIM).unwrap();
            let c1 = chunk("c1", "d1", 0);
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c1, vector: vec_of(1) }]).unwrap();
        } // drop store → release locks before reopening read-side
        let search = RuVectorSearchAdapter::open(&p.vectors, DIM).unwrap();
        let hits = search.vector_search(&vec_of(1), 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "c1");
        assert_eq!(hits[0].document_id, "d1");
    }

    #[test]
    fn upsert_rejects_dimension_mismatch() {
        let p = paths();
        let store = RuVectorStoreAdapter::open(&p.vectors, &p.chunkmap, DIM).unwrap();
        let c = chunk("c1", "d1", 0);
        let items = vec![ChunkWithEmbedding { chunk: &c, vector: vec![0.0; DIM + 1] }];
        let err = store.upsert_chunks(&items).unwrap_err();
        assert!(format!("{err:#}").contains("dim mismatch"), "got: {err:#}");
    }

    #[test]
    fn delete_removes_every_chunk_of_a_doc_across_separate_upserts() {
        // Two upserts for d1 must MERGE in the chunk-id map (not overwrite), so deleting
        // d1 removes both c1 and c2; an unrelated doc d2 is untouched. If the map overwrote
        // instead of appending, c1 would leak and survive the delete.
        let p = paths();
        {
            let store = RuVectorStoreAdapter::open(&p.vectors, &p.chunkmap, DIM).unwrap();
            let (c1, c2, c3) = (chunk("c1", "d1", 0), chunk("c2", "d1", 1), chunk("c3", "d2", 0));
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c1, vector: vec_of(1) }]).unwrap();
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c2, vector: vec_of(2) }]).unwrap();
            store.upsert_chunks(&[ChunkWithEmbedding { chunk: &c3, vector: vec_of(3) }]).unwrap();
            store.delete_by_document(&"d1".to_string()).unwrap();
        }
        let search = RuVectorSearchAdapter::open(&p.vectors, DIM).unwrap();
        let hits = search.vector_search(&vec_of(1), 10).unwrap();
        let ids: Vec<_> = hits.iter().map(|h| h.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c3"], "only d2's chunk should survive; got {ids:?}");
    }

    #[test]
    fn delete_unknown_document_is_a_noop_even_with_no_map_table() {
        // A fresh store has never written the DOC_CHUNKS table; deleting must not error.
        let p = paths();
        let store = RuVectorStoreAdapter::open(&p.vectors, &p.chunkmap, DIM).unwrap();
        store.delete_by_document(&"ghost".to_string()).unwrap();
    }
}
