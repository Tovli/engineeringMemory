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
