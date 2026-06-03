//! RedbDocumentRepository — persists IngestionDocument records in a redb file (D-PERSIST).
//! Keyed by source_path; documents serialized as JSON.

use redb::{Database, ReadableTable, TableDefinition};

use crate::ingestion::domain::{DocumentId, DocumentStatus, IngestionDocument};
use crate::ingestion::ports::DocumentRepository;

const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");

pub struct RedbDocumentRepository {
    db: Database,
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
        // Keyed by path; find the matching record, flip status, re-save.
        let target = self.all()?.into_iter().find(|d| &d.id == id);
        if let Some(mut doc) = target {
            doc.status = DocumentStatus::Deleted;
            self.save(&doc)?;
        }
        Ok(())
    }

    fn active_under(&self, root: &str) -> anyhow::Result<Vec<IngestionDocument>> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|d| d.status != DocumentStatus::Deleted && d.source_path.starts_with(root))
            .collect())
    }
}
