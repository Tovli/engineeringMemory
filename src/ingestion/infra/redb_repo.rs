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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    fn repo_path() -> String {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("tovli-repo-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&base).unwrap();
        base.join("documents.redb").to_string_lossy().to_string()
    }

    fn document(id: &str, path: &str, status: DocumentStatus) -> IngestionDocument {
        IngestionDocument {
            id: id.into(),
            source_path: path.into(),
            file_name: "f.md".into(),
            file_extension: "md".into(),
            content_hash: "h".into(),
            title: None,
            project: None,
            tags: vec![],
            status,
            embedding_model: "m".into(),
            embedding_dimension: 8,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-02T00:00:00Z".into(),
            deleted_at: None,
        }
    }

    #[test]
    fn find_by_path_miss_returns_none() {
        let repo = RedbDocumentRepository::open(&repo_path()).unwrap();
        assert!(repo.find_by_path("docs/nope.md").unwrap().is_none());
    }

    #[test]
    fn save_then_find_round_trips() {
        let repo = RedbDocumentRepository::open(&repo_path()).unwrap();
        repo.save(&document("d1", "docs/a.md", DocumentStatus::Active)).unwrap();
        let got = repo.find_by_path("docs/a.md").unwrap().expect("saved doc found");
        assert_eq!(got.id, "d1");
        assert_eq!(got.status, DocumentStatus::Active);
    }

    #[test]
    fn soft_delete_finds_record_by_id_and_flips_status() {
        // soft_delete keys by id even though the table is keyed by source_path.
        let repo = RedbDocumentRepository::open(&repo_path()).unwrap();
        repo.save(&document("d1", "docs/a.md", DocumentStatus::Active)).unwrap();
        repo.soft_delete(&"d1".to_string()).unwrap();
        let got = repo.find_by_path("docs/a.md").unwrap().unwrap();
        assert_eq!(got.status, DocumentStatus::Deleted);
    }

    #[test]
    fn soft_delete_unknown_id_is_a_noop() {
        let repo = RedbDocumentRepository::open(&repo_path()).unwrap();
        repo.save(&document("d1", "docs/a.md", DocumentStatus::Active)).unwrap();
        repo.soft_delete(&"ghost".to_string()).unwrap();
        assert_eq!(repo.find_by_path("docs/a.md").unwrap().unwrap().status, DocumentStatus::Active);
    }

    #[test]
    fn active_under_filters_by_root_prefix_and_excludes_deleted() {
        let repo = RedbDocumentRepository::open(&repo_path()).unwrap();
        repo.save(&document("d1", "proj_a/a.md", DocumentStatus::Active)).unwrap();
        repo.save(&document("d2", "proj_a/sub/b.md", DocumentStatus::Active)).unwrap();
        repo.save(&document("d3", "proj_b/c.md", DocumentStatus::Active)).unwrap();
        repo.save(&document("d4", "proj_a/old.md", DocumentStatus::Deleted)).unwrap();

        let mut ids: Vec<String> =
            repo.active_under("proj_a").unwrap().into_iter().map(|d| d.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["d1".to_string(), "d2".to_string()], "only active docs under proj_a");
    }
}
