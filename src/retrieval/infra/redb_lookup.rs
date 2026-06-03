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
