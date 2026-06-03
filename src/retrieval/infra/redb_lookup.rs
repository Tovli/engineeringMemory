//! RedbDocumentLookup — read-only DocumentLookupPort over the `documents.redb` file that the
//! M1 ingestion repository writes. Reuses that table layout (keyed by source_path, value = JSON
//! IngestionDocument). Resolves document metadata for the project/tag filter join (ADR-0002) and
//! reports the indexed embedding model for the compatibility guard (R6/AC-7).

use std::collections::HashMap;

use redb::{Database, ReadableTable, TableDefinition};

use crate::ingestion::domain::{DocumentId, DocumentStatus, EmbeddingModelVersion, IngestionDocument};
use crate::retrieval::ports::{DocMeta, DocumentLookupPort};

const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");

pub struct RedbDocumentLookup {
    /// `None` when the documents file does not exist yet → treated as an empty index (AC-8).
    db: Option<Database>,
}

impl RedbDocumentLookup {
    /// Open the documents store read-only. A missing file is not an error: it means nothing has
    /// been ingested yet, which the port reports as an empty index.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        match Database::open(path) {
            Ok(db) => Ok(Self { db: Some(db) }),
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(e)))
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Self { db: None })
            }
            Err(e) => Err(e.into()),
        }
    }

    fn all(&self) -> anyhow::Result<Vec<IngestionDocument>> {
        let Some(db) = &self.db else { return Ok(vec![]) };
        let rtxn = db.begin_read()?;
        let table = match rtxn.open_table(DOCS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for row in table.iter()? {
            let (_, v) = row?;
            out.push(serde_json::from_str(v.value())?);
        }
        Ok(out)
    }
}

impl DocumentLookupPort for RedbDocumentLookup {
    fn find_many(&self, ids: &[DocumentId]) -> anyhow::Result<HashMap<DocumentId, DocMeta>> {
        let wanted: std::collections::HashSet<&DocumentId> = ids.iter().collect();
        let mut out = HashMap::new();
        for d in self.all()? {
            if wanted.contains(&d.id) {
                out.insert(
                    d.id.clone(),
                    DocMeta {
                        project: d.project.clone(),
                        tags: d.tags.clone(),
                        source_path: d.source_path.clone(),
                        status: d.status,
                    },
                );
            }
        }
        Ok(out)
    }

    fn indexed_model_version(&self) -> anyhow::Result<Option<EmbeddingModelVersion>> {
        // The index is what active documents were embedded with; pick any active doc's model.
        Ok(self.all()?.into_iter().find(|d| d.status != DocumentStatus::Deleted).map(|d| {
            EmbeddingModelVersion {
                name: d.embedding_model,
                dimension: d.embedding_dimension,
                created_at: d.updated_at,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::ingestion::infra::redb_repo::RedbDocumentRepository;
    use crate::ingestion::ports::DocumentRepository;

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    fn docs_path() -> String {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-lookup-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        base.join("documents.redb").to_string_lossy().to_string()
    }

    fn document(
        id: &str,
        path: &str,
        status: DocumentStatus,
        model: &str,
        dim: usize,
    ) -> IngestionDocument {
        IngestionDocument {
            id: id.into(),
            source_path: path.into(),
            file_name: "f.md".into(),
            file_extension: "md".into(),
            content_hash: "h".into(),
            title: None,
            project: Some("proj".into()),
            tags: vec!["t".into()],
            status,
            embedding_model: model.into(),
            embedding_dimension: dim,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-02T00:00:00Z".into(),
            deleted_at: None,
        }
    }

    #[test]
    fn missing_file_is_treated_as_an_empty_index() {
        // AC-8: opening a never-created documents.redb is not an error.
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir()
            .join(format!("tovli-missing-{}-{}.redb", std::process::id(), n))
            .to_string_lossy()
            .to_string();
        let lookup = RedbDocumentLookup::open(&path).unwrap();
        assert!(lookup.indexed_model_version().unwrap().is_none());
        assert!(lookup.find_many(&["d1".to_string()]).unwrap().is_empty());
    }

    #[test]
    fn indexed_model_version_ignores_deleted_docs() {
        // R6/AC-7: the guard must report the ACTIVE index's model, never a stale deleted one.
        let path = docs_path();
        {
            let repo = RedbDocumentRepository::open(&path).unwrap();
            repo.save(&document("d_del", "docs/old.md", DocumentStatus::Deleted, "stale-model", 16))
                .unwrap();
            repo.save(&document("d_act", "docs/new.md", DocumentStatus::Active, "live-model", 384))
                .unwrap();
        }
        let lookup = RedbDocumentLookup::open(&path).unwrap();
        let mv = lookup.indexed_model_version().unwrap().expect("an active doc provides the model");
        assert_eq!(mv.name, "live-model");
        assert_eq!(mv.dimension, 384);
    }

    #[test]
    fn all_deleted_means_empty_index() {
        let path = docs_path();
        {
            let repo = RedbDocumentRepository::open(&path).unwrap();
            repo.save(&document("d1", "docs/a.md", DocumentStatus::Deleted, "m", 8)).unwrap();
        }
        let lookup = RedbDocumentLookup::open(&path).unwrap();
        assert!(lookup.indexed_model_version().unwrap().is_none());
    }

    #[test]
    fn find_many_returns_only_requested_ids_with_full_metadata() {
        let path = docs_path();
        {
            let repo = RedbDocumentRepository::open(&path).unwrap();
            repo.save(&document("d1", "docs/a.md", DocumentStatus::Active, "m", 8)).unwrap();
            repo.save(&document("d2", "docs/b.md", DocumentStatus::Active, "m", 8)).unwrap();
        }
        let lookup = RedbDocumentLookup::open(&path).unwrap();
        let got = lookup.find_many(&["d1".to_string()]).unwrap();
        assert_eq!(got.len(), 1);
        let meta = got.get("d1").expect("d1 resolved");
        assert_eq!(meta.source_path, "docs/a.md");
        assert_eq!(meta.project.as_deref(), Some("proj"));
        assert_eq!(meta.tags, vec!["t".to_string()]);
        assert_eq!(meta.status, DocumentStatus::Active);
        assert!(!got.contains_key("d2"), "unrequested id must not appear");
    }
}
