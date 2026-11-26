//! RedbDocumentRepository — persists IngestionDocument records in a redb file (D-PERSIST).
//! Keyed by source_path; documents serialized as JSON.

use redb::{Database, ReadableTable, TableDefinition};

use crate::ingestion::domain::{DocumentId, DocumentStatus, IngestionDocument};
use crate::ingestion::ports::DocumentRepository;

pub(crate) const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");

pub struct RedbDocumentRepository {
    pub(crate) db: Database,
}

impl RedbDocumentRepository {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        Ok(Self { db: Database::create(path)? })
    }

    fn all(&self) -> anyhow::Result<Vec<IngestionDocument>> {
        let rtxn = self.db.begin_read()?;
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

    pub(crate) fn indexed_model(&self) -> anyhow::Result<Option<(String, usize)>> {
        Ok(self.all()?.into_iter().find(|doc| doc.status != DocumentStatus::Deleted)
            .map(|doc| (doc.embedding_model, doc.embedding_dimension)))
    }
}

impl DocumentRepository for RedbDocumentRepository {
    fn find_by_path(&self, path: &str) -> anyhow::Result<Option<IngestionDocument>> {
        let rtxn = self.db.begin_read()?;
        let table = match rtxn.open_table(DOCS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match table.get(path)? {
            Some(g) => Ok(Some(serde_json::from_str(g.value())?)),
            None => Ok(None),
        }
    }

    fn save(&self, doc: &IngestionDocument) -> anyhow::Result<()> {
        let json = serde_json::to_string(doc)?;
        let wtxn = self.db.begin_write()?;
        {
            let mut table = wtxn.open_table(DOCS)?;
            table.insert(doc.source_path.as_str(), json.as_str())?;
        }
        wtxn.commit()?;
        Ok(())
    }

    fn soft_delete(&self, id: &DocumentId) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        let target = {
            let docs = tx.open_table(DOCS)?;
            let mut target = None;
            for row in docs.iter()? {
                let (key, value) = row?;
                let mut doc: serde_json::Value = serde_json::from_str(value.value())?;
                if doc.get("id").and_then(|v| v.as_str()) == Some(id) {
                    doc["status"] = serde_json::json!("Deleted");
                    target = Some((key.value().to_owned(), serde_json::to_string(&doc)?));
                    break;
                }
            }
            target
        };
        if let Some((path, document)) = target {
            {
                let mut docs = tx.open_table(DOCS)?;
                docs.insert(path.as_str(), document.as_str())?;
            }
            crate::image_memory::links::suppress_for_document(&tx, id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn active_under(&self, root: &str) -> anyhow::Result<Vec<IngestionDocument>> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|d| {
                d.status != DocumentStatus::Deleted
                    && !d.source_path.starts_with("mcp://")
                    && std::path::Path::new(&d.source_path).starts_with(root)
            })
            .collect())
    }
}
