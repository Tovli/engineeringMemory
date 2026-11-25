//! RedbDocumentLookup — read-only DocumentLookupPort over the `documents.redb` file that the
//! M1 ingestion repository writes. Reuses that table layout (keyed by source_path, value = JSON
//! IngestionDocument). Resolves document metadata for the project/tag filter join (ADR-0002) and
//! reports the indexed embedding model for the compatibility guard (R6/AC-7).

use std::collections::HashMap;

use redb::{Database, ReadableTable, TableDefinition};

use crate::ingestion::domain::{DocumentId, DocumentStatus, EmbeddingModelVersion, IngestionDocument};
use crate::retrieval::ports::{DocMeta, DocumentLookupPort, ImageAttachment};

const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");

pub struct RedbDocumentLookup {
    /// `None` when the documents file does not exist yet → treated as an empty index (AC-8).
    db: Option<Database>,
    store_dir: std::path::PathBuf,
}

impl RedbDocumentLookup {
    /// Open the documents store read-only. A missing file is not an error: it means nothing has
    /// been ingested yet, which the port reports as an empty index.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let store_dir = std::path::Path::new(path).parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf();
        match Database::open(path) {
            Ok(db) => Ok(Self { db: Some(db), store_dir }),
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(e)))
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Self { db: None, store_dir })
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
    fn image_attachments(
        &self,
        results: &[(String, String)],
        project: Option<&str>,
        tags: &[String],
    ) -> anyhow::Result<HashMap<String, (Vec<ImageAttachment>, bool)>> {
        let Some(db) = &self.db else { return Ok(HashMap::new()) };
        let mut output = HashMap::new();
        for (document_id, chunk_id) in results {
            let links = crate::image_memory::links::list(db, document_id)?;
            let target_ids: Vec<_> = links.iter().map(|link| link.image_document_id.clone()).collect();
            let targets = self.find_many(&target_ids)?;
            let mut attachments = Vec::new();
            for link in links {
                if link.document_id != *document_id
                    || link.chunk_id.as_ref().is_some_and(|id| id != chunk_id) {
                    continue;
                }
                let Some(target) = targets.get(&link.image_document_id) else { continue };
                if project.is_some_and(|p| target.project.as_deref() != Some(p))
                    || !tags.iter().all(|tag| target.tags.contains(tag)) {
                    continue;
                }
                attachments.push(ImageAttachment {
                    link_id: link.link_id,
                    image_document_id: link.image_document_id,
                    asset_id: link.asset_id,
                    caption: link.caption,
                    anchor_state: link.anchor_state,
                });
            }
            let more = attachments.len() > 20;
            attachments.truncate(20);
            if !attachments.is_empty() {
                output.insert(chunk_id.clone(), (attachments, more));
            }
        }
        Ok(output)
    }

    fn image_assets(&self, ids: &[DocumentId]) -> anyhow::Result<HashMap<DocumentId, (String, bool)>> {
        let Some(db) = &self.db else { return Ok(HashMap::new()) };
        let wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        let tx = db.begin_read()?;
        let docs = match tx.open_table(DOCS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(HashMap::new()),
            Err(error) => return Err(error.into()),
        };
        let assets = match tx.open_table(crate::ingestion::durable::ASSETS) {
            Ok(table) => Some(table),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(error.into()),
        };
        let mut result = HashMap::new();
        for row in docs.iter()? {
            let (_, value) = row?;
            let doc: serde_json::Value = serde_json::from_str(value.value())?;
            let Some(id) = doc.get("id").and_then(|v| v.as_str()) else { continue };
            if !wanted.contains(id) || doc.get("source_kind").and_then(|v| v.as_str()) != Some("image") {
                continue;
            }
            let Some(asset_id) = doc.get("asset_id").and_then(|v| v.as_str()) else { continue };
            let available = match &assets {
                Some(table) => match table.get(asset_id)? {
                    Some(value) => {
                        let asset: crate::image_memory::assets::ImageAsset = serde_json::from_str(value.value())?;
                        crate::image_memory::assets::is_present(&self.store_dir, &asset)?
                    }
                    None => false,
                },
                None => false,
            };
            result.insert(id.to_owned(), (asset_id.to_owned(), available));
        }
        Ok(result)
    }

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
