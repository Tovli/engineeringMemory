//! Explicit image attachments stored in the document metadata database.
//!
//! Callers hold the shared store lock and provide current chunk membership.

use anyhow::{bail, Result};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use crate::ingestion::durable::recover_locked;
use crate::ingestion::infra::redb_repo::RedbDocumentRepository;
use crate::store_lock::StoreLock;

const DOCUMENTS: TableDefinition<&str, &str> = TableDefinition::new("documents");
const LINKS: TableDefinition<&str, &str> = TableDefinition::new("image_links");
const DOC_CHUNKS: TableDefinition<&str, &str> = TableDefinition::new("doc_chunks");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageLink {
    pub link_id: String,
    pub document_id: String,
    pub image_document_id: String,
    pub asset_id: String,
    pub chunk_id: Option<String>,
    pub caption: Option<String>,
    pub created_at: String,
    pub state: String,
    pub anchor_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkInput {
    pub document_id: String,
    pub image_document_id: String,
    pub chunk_id: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub image_document_id: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 20 }

#[derive(Debug, Serialize)]
pub struct LinkPage {
    pub results: Vec<ImageLink>,
    pub next_cursor: Option<String>,
}

fn find_document(db: &Database, id: &str) -> Result<Option<serde_json::Value>> {
    let txn = db.begin_read()?;
    let table = match txn.open_table(DOCUMENTS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    for row in table.iter()? {
        let (_, value) = row?;
        let doc: serde_json::Value = serde_json::from_str(value.value())?;
        if doc.get("id").and_then(|v| v.as_str()) == Some(id) {
            return Ok(Some(doc));
        }
    }
    Ok(None)
}

fn active(doc: &serde_json::Value) -> bool {
    doc.get("status").and_then(|v| v.as_str()) != Some("Deleted")
}

/// Link two active documents. The caller passes current chunk IDs of the text document.
pub fn create(
    db: &Database,
    input: &LinkInput,
    current_chunk_ids: &[String],
    now: &str,
) -> Result<(ImageLink, bool)> {
    if input.caption.as_ref().is_some_and(|caption| caption.chars().count() > 2048) {
        bail!("invalid_caption");
    }
    let text = find_document(db, &input.document_id)?.ok_or_else(|| anyhow::anyhow!("document_not_found"))?;
    let image = find_document(db, &input.image_document_id)?.ok_or_else(|| anyhow::anyhow!("document_not_found"))?;
    if !active(&text) || !active(&image) {
        bail!("document_not_found");
    }
    if text.get("source_kind").and_then(|v| v.as_str()).unwrap_or("text") != "text"
        || image.get("source_kind").and_then(|v| v.as_str()) != Some("image")
    {
        bail!("invalid_document_kind");
    }
    if input.chunk_id.as_ref().is_some_and(|id| !current_chunk_ids.contains(id)) {
        bail!("invalid_anchor");
    }
    let asset_id = image.get("asset_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("asset_missing"))?.to_owned();
    let tuple = serde_json::to_vec(&(&input.document_id, &input.image_document_id, &input.chunk_id))?;
    let link_id = format!("link_{}", blake3::hash(&tuple).to_hex());
    let txn = db.begin_write()?;
    let (link, created) = {
        let mut table = txn.open_table(LINKS)?;
        let existing = table.get(link_id.as_str())?
            .map(|value| serde_json::from_str::<ImageLink>(value.value()))
            .transpose()?;
        if let Some(existing) = existing {
            if existing.state != "active" {
                bail!("link_inactive");
            }
            if existing.caption != input.caption {
                bail!("link_conflict");
            }
            (existing, false)
        } else {
            let link = ImageLink {
                link_id: link_id.clone(),
                document_id: input.document_id.clone(),
                image_document_id: input.image_document_id.clone(),
                asset_id,
                chunk_id: input.chunk_id.clone(),
                caption: input.caption.clone(),
                created_at: now.to_owned(),
                state: "active".into(),
                anchor_state: if input.chunk_id.is_some() { "current" } else { "document" }.into(),
                suppressed_at: None,
                suppression_reason: None,
            };
            let json = serde_json::to_string(&link)?;
            table.insert(link_id.as_str(), json.as_str())?;
            (link, true)
        }
    };
    txn.commit()?;
    Ok((link, created))
}

/// Mark relationships inactive in the same transaction that deletes a document.
pub(crate) fn suppress_for_document(txn: &redb::WriteTransaction, document_id: &str) -> Result<()> {
    let mut table = txn.open_table(LINKS)?;
    let mut updates = Vec::new();
    for row in table.iter()? {
        let (key, value) = row?;
        let mut link: ImageLink = serde_json::from_str(value.value())?;
        if link.state == "active"
            && (link.document_id == document_id || link.image_document_id == document_id)
        {
            link.state = "suppressed".into();
            link.suppressed_at = Some(chrono::Utc::now().to_rfc3339());
            link.suppression_reason = Some("endpoint_deleted".into());
            updates.push((key.value().to_owned(), serde_json::to_string(&link)?));
        }
    }
    for (key, value) in updates {
        table.insert(key.as_str(), value.as_str())?;
    }
    Ok(())
}

/// Return active links for one endpoint. IDs sort deterministically for pagination.
pub fn list(db: &Database, document_id: &str) -> Result<Vec<ImageLink>> {
    let txn = db.begin_read()?;
    let table = match txn.open_table(LINKS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut links = Vec::new();
    for row in table.iter()? {
        let (_, value) = row?;
        let link: ImageLink = serde_json::from_str(value.value())?;
        if link.state == "active"
            && (link.document_id == document_id || link.image_document_id == document_id)
        {
            links.push(link);
        }
    }
    drop(table);
    drop(txn);
    let mut active_links = Vec::new();
    for link in links {
        let text = find_document(db, &link.document_id)?;
        let image = find_document(db, &link.image_document_id)?;
        if text.as_ref().is_some_and(active) && image.as_ref().is_some_and(active) {
            active_links.push(link);
        }
    }
    Ok(active_links)
}

fn chunk_ids(root: &Path, document_id: &str) -> Result<Vec<String>> {
    let map = Database::create(root.join("chunkmap.redb"))?;
    let txn = map.begin_read()?;
    let table = match txn.open_table(DOC_CHUNKS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let ids = table.get(document_id)?.map(|v| serde_json::from_str(v.value())).transpose()?;
    Ok(ids.unwrap_or_default())
}

/// Create a link while holding the shared store lock.
pub fn create_for_store(root: &Path, input: &LinkInput) -> Result<(ImageLink, bool)> {
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    recover_locked(&lock.path)?;
    let repo = RedbDocumentRepository::open(&lock.path.join("documents.redb").to_string_lossy())?;
    let image = find_document(&repo.db, &input.image_document_id)?
        .ok_or_else(|| anyhow::anyhow!("document_not_found"))?;
    let asset_id = image.get("asset_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("asset_missing"))?;
    {
        let tx = repo.db.begin_read()?;
        let assets = tx.open_table(crate::ingestion::durable::ASSETS)?;
        let value = assets.get(asset_id)?.ok_or_else(|| anyhow::anyhow!("asset_missing"))?;
        let asset: super::assets::ImageAsset = serde_json::from_str(value.value())?;
        super::assets::read(&lock.path, &asset)?;
    }
    let chunks = chunk_ids(&lock.path, &input.document_id)?;
    create(&repo.db, input, &chunks, &chrono::Utc::now().to_rfc3339())
}

/// Look up relationships while holding the shared store lock.
pub fn list_for_store(root: &Path, document_id: &str) -> Result<Vec<ImageLink>> {
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    recover_locked(&lock.path)?;
    let repo = RedbDocumentRepository::open(&lock.path.join("documents.redb").to_string_lossy())?;
    let mut links = list(&repo.db, document_id)?;
    for link in &mut links {
        if let Some(id) = &link.chunk_id {
            if !chunk_ids(&lock.path, &link.document_id)?.contains(id) {
                link.anchor_state = "stale".into();
            }
        }
    }
    Ok(links)
}

/// List one endpoint's links, filtering related targets and binding cursors to the query.
pub fn list_page_for_store(root: &Path, mut input: ListInput) -> Result<LinkPage> {
    let (endpoint, image_endpoint) = match (&input.document_id, &input.image_document_id) {
        (Some(id), None) if !id.is_empty() => (id.clone(), false),
        (None, Some(id)) if !id.is_empty() => (id.clone(), true),
        _ => bail!("invalid_input"),
    };
    if input.limit == 0 || input.limit > 100 || input.tags.len() > 32
        || input.project.as_ref().is_some_and(|p| p.chars().count() > 128)
        || input.tags.iter().any(|tag| tag.trim().is_empty() || tag.chars().count() > 64)
    {
        bail!("invalid_input");
    }
    input.tags.sort();
    input.tags.dedup();
    let query = serde_json::to_vec(&(&endpoint, image_endpoint, &input.project, &input.tags))?;
    let scope_hash = blake3::hash(&query).to_hex().to_string();
    let after = if let Some(cursor) = &input.cursor {
        let (scope, id) = cursor.split_once('.').ok_or_else(|| anyhow::anyhow!("invalid_cursor"))?;
        if scope != scope_hash || id.len() != 69 || !id.starts_with("link_")
            || !id[5..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("invalid_cursor");
        }
        Some(id.to_owned())
    } else {
        None
    };
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    recover_locked(&lock.path)?;
    let repo = RedbDocumentRepository::open(&lock.path.join("documents.redb").to_string_lossy())?;
    let doc = find_document(&repo.db, &endpoint)?
        .ok_or_else(|| anyhow::anyhow!("document_not_found"))?;
    let kind = doc.get("source_kind").and_then(|v| v.as_str()).unwrap_or("text");
    if !active(&doc) || (image_endpoint && kind != "image") || (!image_endpoint && kind != "text") {
        bail!("document_not_found");
    }
    let mut matches = Vec::new();
    for mut link in list(&repo.db, &endpoint)? {
        if after.as_ref().is_some_and(|id| link.link_id <= *id) {
            continue;
        }
        let target_id = if image_endpoint { &link.document_id } else { &link.image_document_id };
        let Some(target) = find_document(&repo.db, target_id)? else { continue };
        if input.project.as_ref().is_some_and(|p| target.get("project").and_then(|v| v.as_str()) != Some(p)) {
            continue;
        }
        let target_tags = target.get("tags").and_then(|v| v.as_array());
        if !input.tags.iter().all(|tag| target_tags.is_some_and(|items| items.iter().any(|v| v.as_str() == Some(tag)))) {
            continue;
        }
        if let Some(id) = &link.chunk_id {
            if !chunk_ids(&lock.path, &link.document_id)?.contains(id) {
                link.anchor_state = "stale".into();
            }
        }
        matches.push(link);
        if matches.len() > input.limit {
            break;
        }
    }
    let more = matches.len() > input.limit;
    matches.truncate(input.limit);
    let next_cursor = if more { matches.last().map(|link| format!("{scope_hash}.{}", link.link_id)) } else { None };
    Ok(LinkPage { results: matches, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::ports::DocumentRepository;
    use image::{DynamicImage, ImageOutputFormat, RgbaImage};
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn links_are_idempotent_and_suppress_deleted_endpoints() {
        let path = std::env::temp_dir().join(format!(
            "tovli-links-{}-{}.redb",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let db = Database::create(&path).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut docs = tx.open_table(DOCUMENTS).unwrap();
            docs.insert("text", r#"{"id":"text","status":"Active"}"#).unwrap();
            docs.insert("image", r#"{"id":"image","status":"Active","source_kind":"image","asset_id":"asset_example"}"#).unwrap();
        }
        tx.commit().unwrap();
        let input = LinkInput {
            document_id: "text".into(),
            image_document_id: "image".into(),
            chunk_id: Some("chunk".into()),
            caption: Some("diagram".into()),
        };
        let chunk_ids = vec!["chunk".into()];
        let (link, created) = create(&db, &input, &chunk_ids, "now").unwrap();
        assert!(created);
        let (retry, created) = create(&db, &input, &chunk_ids, "later").unwrap();
        assert!(!created);
        assert_eq!(link.link_id, retry.link_id);
        assert_eq!(retry.created_at, "now");
        let mut conflict = input.clone();
        conflict.caption = Some("changed".into());
        assert!(create(&db, &conflict, &chunk_ids, "later").is_err());
        assert!(create(&db, &input, &[], "later").is_err());
        assert_eq!(list(&db, "text").unwrap().len(), 1);
        assert_eq!(list(&db, "image").unwrap().len(), 1);
        let tx = db.begin_write().unwrap();
        {
            let mut docs = tx.open_table(DOCUMENTS).unwrap();
            docs.insert("image", r#"{"id":"image","status":"Deleted","source_kind":"image","asset_id":"asset_example"}"#).unwrap();
        }
        tx.commit().unwrap();
        assert!(list(&db, "text").unwrap().is_empty());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn paginates_filters_marks_stale_and_suppresses_on_delete() {
        let root = std::env::temp_dir().join(format!(
            "tovli-link-pages-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(2, 2))
            .write_to(&mut bytes, ImageOutputFormat::Png).unwrap();
        let asset = super::super::assets::publish(&root, &bytes.into_inner(), "image/png").unwrap();
        let db = Database::create(root.join("documents.redb")).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut docs = tx.open_table(DOCUMENTS).unwrap();
            for (id, kind, project) in [("t1", "text", "alpha"), ("t2", "text", "alpha"), ("t3", "text", "beta"), ("img", "image", "alpha")] {
                let doc = serde_json::json!({
                    "id": id, "source_path": id, "status": "Active",
                    "source_kind": kind, "asset_id": asset.asset_id,
                    "project": project, "tags": ["one", "two"]
                });
                docs.insert(id, doc.to_string().as_str()).unwrap();
            }
            let mut assets = tx.open_table(crate::ingestion::durable::ASSETS).unwrap();
            assets.insert(asset.asset_id.as_str(), serde_json::to_string(&asset).unwrap().as_str()).unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let chunk_db = Database::create(root.join("chunkmap.redb")).unwrap();
        let tx = chunk_db.begin_write().unwrap();
        {
            let mut chunks = tx.open_table(DOC_CHUNKS).unwrap();
            chunks.insert("t1", r#"["chunk"]"#).unwrap();
        }
        tx.commit().unwrap();
        drop(chunk_db);
        for (id, chunk) in [("t1", Some("chunk")), ("t2", None), ("t3", None)] {
            create_for_store(&root, &LinkInput {
                document_id: id.into(), image_document_id: "img".into(),
                chunk_id: chunk.map(str::to_owned), caption: Some(id.into()),
            }).unwrap();
        }
        let mut filter = ListInput {
            document_id: None, image_document_id: Some("img".into()),
            project: Some("alpha".into()), tags: vec!["two".into(), "one".into()],
            cursor: None, limit: 1,
        };
        let first = list_page_for_store(&root, filter.clone()).unwrap();
        assert_eq!(first.results.len(), 1);
        filter.cursor = first.next_cursor.clone();
        let second = list_page_for_store(&root, filter.clone()).unwrap();
        assert_eq!(second.results.len(), 1);
        assert_ne!(first.results[0].link_id, second.results[0].link_id);
        assert!(second.next_cursor.is_none());
        filter.project = Some("beta".into());
        assert!(list_page_for_store(&root, filter.clone()).is_err());

        let chunk_db = Database::create(root.join("chunkmap.redb")).unwrap();
        let tx = chunk_db.begin_write().unwrap();
        {
            let mut chunks = tx.open_table(DOC_CHUNKS).unwrap();
            chunks.insert("t1", "[]").unwrap();
        }
        tx.commit().unwrap();
        drop(chunk_db);
        let listing = list_for_store(&root, "t1").unwrap();
        assert_eq!(listing[0].anchor_state, "stale");

        // Deleting and later restoring an endpoint must not restore its links.
        let repo = RedbDocumentRepository::open(&root.join("documents.redb").to_string_lossy()).unwrap();
        repo.soft_delete(&"t1".to_owned()).unwrap();
        let tx = repo.db.begin_write().unwrap();
        {
            let mut docs = tx.open_table(DOCUMENTS).unwrap();
            let doc: serde_json::Value = serde_json::from_str(docs.get("t1").unwrap().unwrap().value()).unwrap();
            assert_eq!(doc["status"], "Deleted");
            assert_eq!(doc["source_kind"], "text");
            let mut restored = doc;
            restored["status"] = serde_json::json!("Active");
            docs.insert("t1", restored.to_string().as_str()).unwrap();
        }
        tx.commit().unwrap();
        assert!(list(&repo.db, "t1").unwrap().is_empty());
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
}
