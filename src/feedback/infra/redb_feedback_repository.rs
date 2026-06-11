//! redb-backed feedback repository. Items are JSON values keyed by feedback id.

use std::path::Path;

use redb::{Database, ReadableTable, TableDefinition};

use crate::feedback::domain::{FeedbackItem, FeedbackQuery};
use crate::feedback::ports::FeedbackRepository;

const FEEDBACK: TableDefinition<&str, &str> = TableDefinition::new("feedback");

pub struct RedbFeedbackRepository {
    db: Database,
}

impl RedbFeedbackRepository {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            db: Database::create(path)?,
        })
    }

    pub fn save(&self, item: &FeedbackItem) -> anyhow::Result<()> {
        <Self as FeedbackRepository>::save(self, item)
    }

    pub fn find_by_id(&self, id: &str) -> anyhow::Result<Option<FeedbackItem>> {
        <Self as FeedbackRepository>::find_by_id(self, id)
    }

    pub fn find_all(&self, query: Option<FeedbackQuery>) -> anyhow::Result<Vec<FeedbackItem>> {
        <Self as FeedbackRepository>::find_all(self, query)
    }
}

impl FeedbackRepository for RedbFeedbackRepository {
    fn save(&self, item: &FeedbackItem) -> anyhow::Result<()> {
        let json = serde_json::to_string(item)?;
        let wtxn = self.db.begin_write()?;
        {
            let mut table = wtxn.open_table(FEEDBACK)?;
            if table.get(item.id.as_str())?.is_some() {
                anyhow::bail!("feedback id '{}' already exists", item.id);
            }
            table.insert(item.id.as_str(), json.as_str())?;
        }
        wtxn.commit()?;
        Ok(())
    }

    fn find_by_id(&self, id: &str) -> anyhow::Result<Option<FeedbackItem>> {
        let rtxn = self.db.begin_read()?;
        let table = match rtxn.open_table(FEEDBACK) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        match table.get(id)? {
            Some(v) => Ok(Some(serde_json::from_str(v.value())?)),
            None => Ok(None),
        }
    }

    fn find_all(&self, query: Option<FeedbackQuery>) -> anyhow::Result<Vec<FeedbackItem>> {
        let rtxn = self.db.begin_read()?;
        let table = match rtxn.open_table(FEEDBACK) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for row in table.iter()? {
            let (_, v) = row?;
            let item: FeedbackItem = serde_json::from_str(v.value())?;
            if query.as_ref().map(|q| q.matches(&item)).unwrap_or(true) {
                out.push(item);
            }
        }
        out.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }
}

pub fn export_feedback_json(path: &str, repo: &RedbFeedbackRepository) -> anyhow::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let items = repo.find_all(None)?;
    let json = serde_json::to_string_pretty(&items)?;
    std::fs::write(path, json)?;
    Ok(())
}
