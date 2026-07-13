//! redb-backed feedback repository. Items are JSON values keyed by feedback id.

use std::path::Path;

use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashSet;

use crate::feedback::domain::{FeedbackItem, FeedbackQuery};
use crate::feedback::ports::FeedbackRepository;

const FEEDBACK: TableDefinition<&str, &str> = TableDefinition::new("feedback");

pub struct RedbFeedbackRepository {
    db: Database,
    #[cfg(test)]
    fail_after_insert: Option<usize>,
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
            #[cfg(test)]
            fail_after_insert: None,
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
        self.save_many(std::slice::from_ref(item))
    }

    fn save_many(&self, items: &[FeedbackItem]) -> anyhow::Result<()> {
        let mut ids = HashSet::new();
        for item in items {
            if !ids.insert(item.id.as_str()) {
                anyhow::bail!(
                    "feedback id '{}' appears more than once in this batch",
                    item.id
                );
            }
        }
        let wtxn = self.db.begin_write()?;
        {
            let mut table = wtxn.open_table(FEEDBACK)?;
            for item in items {
                if table.get(item.id.as_str())?.is_some() {
                    anyhow::bail!("feedback id '{}' already exists", item.id);
                }
            }
            #[cfg(test)]
            let mut inserted = 0usize;
            for item in items {
                let json = serde_json::to_string(item)?;
                table.insert(item.id.as_str(), json.as_str())?;
                #[cfg(test)]
                {
                    inserted += 1;
                    if self.fail_after_insert == Some(inserted) {
                        anyhow::bail!("injected redb insert failure");
                    }
                }
            }
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::feedback::domain::FeedbackRating;
    use crate::retrieval::domain::SearchMode;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn feedback_item(id: &str) -> FeedbackItem {
        FeedbackItem {
            id: id.into(),
            query_id: "qry_1".into(),
            retrieval_run_id: "rrun_1".into(),
            chunk_id: "chunk_1".into(),
            document_id: "doc_1".into(),
            rating: FeedbackRating::Good,
            note: None,
            search_mode: SearchMode::Hybrid,
            rank: 1,
            score: 0.91,
            source_path: "docs/deploy.md".into(),
            question_text: "why did deployment fail?".into(),
            created_at: "2026-06-10T00:00:00Z".into(),
        }
    }

    #[test]
    fn save_many_rolls_back_when_an_insert_fails() {
        let number = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "tovli-redb-rollback-{}-{number}.redb",
            std::process::id()
        ));
        let mut repo = RedbFeedbackRepository::open(path.to_str().unwrap()).unwrap();
        repo.fail_after_insert = Some(1);

        let err = repo
            .save_many(&[feedback_item("fb_1"), feedback_item("fb_2")])
            .unwrap_err();

        assert!(err.to_string().contains("injected redb insert failure"));
        assert!(repo.find_all(None).unwrap().is_empty());
    }
}
