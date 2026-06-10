//! RedbKeywordIndex — ingestion-side writer for the local keyword index (M5 / ADR-0009).

use redb::{Database, ReadableTable};

use crate::ingestion::domain::DocumentId;
use crate::ingestion::ports::KeywordIndexPort;
use crate::lexical_index::{KeywordIndexedChunk, KEYWORD_CHUNKS};

pub struct RedbKeywordIndex {
    db: Database,
}

impl RedbKeywordIndex {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        Ok(Self {
            db: Database::create(path)?,
        })
    }
}

impl KeywordIndexPort for RedbKeywordIndex {
    fn upsert_chunks(&self, chunks: &[&crate::ingestion::domain::Chunk]) -> anyhow::Result<()> {
        let wtxn = self.db.begin_write()?;
        {
            let mut table = wtxn.open_table(KEYWORD_CHUNKS)?;
            for chunk in chunks {
                let record = KeywordIndexedChunk::from_chunk(chunk);
                let json = serde_json::to_string(&record)?;
                table.insert(record.chunk_id.as_str(), json.as_str())?;
            }
        }
        wtxn.commit()?;
        Ok(())
    }

    fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()> {
        let mut to_remove = Vec::new();
        {
            let rtxn = self.db.begin_read()?;
            let table = match rtxn.open_table(KEYWORD_CHUNKS) {
                Ok(t) => t,
                Err(redb::TableError::TableDoesNotExist(_)) => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            for row in table.iter()? {
                let (k, v) = row?;
                let record: KeywordIndexedChunk = serde_json::from_str(v.value())?;
                if &record.document_id == id {
                    to_remove.push(k.value().to_string());
                }
            }
        }

        if to_remove.is_empty() {
            return Ok(());
        }
        let wtxn = self.db.begin_write()?;
        {
            let mut table = wtxn.open_table(KEYWORD_CHUNKS)?;
            for key in to_remove {
                table.remove(key.as_str())?;
            }
        }
        wtxn.commit()?;
        Ok(())
    }
}
