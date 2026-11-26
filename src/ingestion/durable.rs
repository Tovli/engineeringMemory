//! Durable create-only ingestion for logical sources.
use std::path::Path;
use std::time::Duration;

use redb::{ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::chunking::ChunkingService;
use super::domain::{
    content_hash, document_id, Chunk, ChunkingConfig, DocumentStatus, IngestionDocument,
};
use super::infra::parsers::default_parsers;
use super::infra::redb_keyword_index::RedbKeywordIndex;
use super::infra::redb_repo::{RedbDocumentRepository, DOCS};
use super::infra::ruvector_store::RuVectorStoreAdapter;
use super::ports::{DocumentRepository, Embedder};
use super::processing::{persist_chunks, prepare_chunks, remove_document_chunks};
use crate::image_memory::assets::{self, ImageAsset};
use crate::store_lock::StoreLock;

const RECEIPTS: TableDefinition<&str, &str> = TableDefinition::new("mcp_receipts");
const JOURNAL: TableDefinition<&str, &str> = TableDefinition::new("mcp_journal");
pub(crate) const ASSETS: TableDefinition<&str, &str> = TableDefinition::new("image_assets");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub source_key: String,
    pub content: String,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}
fn default_format() -> String {
    "markdown".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub status: String,
    pub document_id: String,
    pub source_uri: String,
    pub content_hash: String,
    pub chunk_count: usize,
    pub embedding_model: String,
    pub embedding_dimension: usize,
    pub indexed_at: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    fingerprint: String,
    receipt: Receipt,
    source_key: String,
    metadata: Map<String, Value>,
    preview: String,
    #[serde(default)]
    image: Option<ImageAsset>,
    #[serde(default)]
    image_details: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    stored: Stored,
    document: IngestionDocument,
    chunks: Vec<Chunk>,
    vectors: Vec<Vec<f32>>,
    #[serde(default)]
    chunking: ChunkingConfig,
    #[serde(default)]
    image_bytes: Option<Vec<u8>>,
    #[serde(default)]
    image_stage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInput {
    pub source_key: String,
    pub mime_type: String,
    pub description: String,
    #[serde(default)]
    pub ocr_text: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub field_provenance: Option<Value>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn path(root: &Path, name: &str) -> String {
    root.join(name).to_string_lossy().into_owned()
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn validate(input: &Input) -> anyhow::Result<()> {
    let key = input.source_key.as_bytes();
    if key.is_empty()
        || key.len() > 128
        || !key[0].is_ascii_alphanumeric()
        || key
            .iter()
            .any(|b| !b.is_ascii_alphanumeric() && *b != b'_' && *b != b'-')
    {
        anyhow::bail!("invalid_source_key");
    }
    if input.content.trim().is_empty()
        || input.content.len() > 256 * 1024
        || !matches!(input.format.as_str(), "markdown" | "text")
        || input
            .title
            .as_ref()
            .is_some_and(|s| s.chars().count() > 256)
        || input
            .project
            .as_ref()
            .is_some_and(|s| s.chars().count() > 128)
        || input.tags.len() > 32
        || input
            .tags
            .iter()
            .any(|s| s.trim().is_empty() || s.chars().count() > 64)
        || serde_json::to_vec(&input.metadata)?.len() > 4096
    {
        anyhow::bail!("invalid_input");
    }
    Ok(())
}

fn read_record(repo: &RedbDocumentRepository, key: &str) -> anyhow::Result<Option<Stored>> {
    let tx = repo.db.begin_read()?;
    let table = match tx.open_table(RECEIPTS) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let result = table
        .get(key)?
        .map(|v| serde_json::from_str(v.value()))
        .transpose()?;
    Ok(result)
}

fn save_pending(repo: &RedbDocumentRepository, pending: &Pending) -> anyhow::Result<()> {
    let tx = repo.db.begin_write()?;
    {
        let mut table = tx.open_table(JOURNAL)?;
        table.insert(
            pending.stored.source_key.as_str(),
            serde_json::to_string(pending)?.as_str(),
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn finish(repo: &RedbDocumentRepository, pending: &Pending) -> anyhow::Result<()> {
    let tx = repo.db.begin_write()?;
    {
        let mut docs = tx.open_table(DOCS)?;
        let mut document = serde_json::to_value(&pending.document)?;
        if let Some(asset) = &pending.stored.image {
            document["source_kind"] = json!("image");
            document["asset_id"] = json!(asset.asset_id);
            if let Some(details) = &pending.stored.image_details {
                document["image_details"] = details.clone();
            }
            let mut assets = tx.open_table(ASSETS)?;
            assets.insert(
                asset.asset_id.as_str(),
                serde_json::to_string(asset)?.as_str(),
            )?;
        }
        docs.insert(
            pending.document.source_path.as_str(),
            serde_json::to_string(&document)?.as_str(),
        )?;
        let mut receipts = tx.open_table(RECEIPTS)?;
        receipts.insert(
            pending.stored.source_key.as_str(),
            serde_json::to_string(&pending.stored)?.as_str(),
        )?;
        let mut journal = tx.open_table(JOURNAL)?;
        journal.remove(pending.stored.source_key.as_str())?;
    }
    tx.commit()?;
    Ok(())
}

fn apply(root: &Path, repo: &RedbDocumentRepository, pending: &Pending) -> anyhow::Result<()> {
    if let Some((name, dimension)) = repo.indexed_model()? {
        if name != pending.document.embedding_model
            || dimension != pending.document.embedding_dimension
        {
            anyhow::bail!("recovery_required: indexed embedding model mismatch");
        }
    }
    pending.chunking.validate().map_err(anyhow::Error::msg)?;
    if pending.document.embedding_model.is_empty()
        || pending.document.embedding_dimension == 0
        || pending.stored.receipt.embedding_model != pending.document.embedding_model
        || pending.stored.receipt.embedding_dimension != pending.document.embedding_dimension
        || pending.stored.receipt.document_id != pending.document.id
        || pending.stored.receipt.source_uri != pending.document.source_path
        || pending.chunks.is_empty()
        || pending.chunks.len() != pending.vectors.len()
        || pending.chunks.len() != pending.stored.receipt.chunk_count
        || pending
            .chunks
            .iter()
            .any(|chunk| chunk.document_id != pending.document.id)
        || pending.vectors.iter().any(|vector| {
            vector.len() != pending.document.embedding_dimension
                || vector.iter().any(|x| !x.is_finite())
        })
    {
        anyhow::bail!("recovery_required: invalid pending operation");
    }
    let unique: std::collections::HashSet<_> =
        pending.chunks.iter().map(|chunk| &chunk.id).collect();
    if unique.len() != pending.chunks.len() {
        anyhow::bail!("recovery_required: duplicate pending chunk ids");
    }
    if let Some(asset) = &pending.stored.image {
        if let Some(stage) = &pending.image_stage {
            assets::publish_staged(root, asset, stage)?;
        } else {
            let bytes = pending
                .image_bytes
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("recovery_required: missing staged image"))?;
            let published = assets::publish(root, bytes, &asset.mime_type)?;
            if &published != asset {
                anyhow::bail!("recovery_required: image hash mismatch");
            }
        }
    }
    let store = RuVectorStoreAdapter::open(
        &path(root, "vectors.redb"),
        &path(root, "chunkmap.redb"),
        pending.document.embedding_dimension,
    )?;
    let keyword = RedbKeywordIndex::open(&path(root, "keyword.redb"))?;
    let ids: Vec<_> = pending.chunks.iter().map(|c| c.id.clone()).collect();
    store.delete_chunks(&ids)?;
    remove_document_chunks(&store, Some(&keyword), &pending.document.id)?;
    persist_chunks(
        &store,
        Some(&keyword),
        &pending.chunks,
        pending.vectors.clone(),
    )?;
    finish(repo, pending)
}

/// Recover while the caller holds the store lock.
pub fn recover_locked(root: &Path) -> anyhow::Result<()> {
    let repo = RedbDocumentRepository::open(&path(root, "documents.redb"))?;
    let pending: Vec<Pending> = (|| -> anyhow::Result<Vec<Pending>> {
        let tx = repo.db.begin_read()?;
        let entries = match tx.open_table(JOURNAL) {
            Ok(table) => table
                .iter()?
                .map(|r| {
                    let (_, value) = r?;
                    Ok(serde_json::from_str(value.value())?)
                })
                .collect::<anyhow::Result<_>>()?,
            Err(redb::TableError::TableDoesNotExist(_)) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(entries)
    })()
    .map_err(|e| anyhow::anyhow!("recovery_required: {e}"))?;
    for entry in pending {
        apply(root, &repo, &entry).map_err(|e| anyhow::anyhow!("recovery_required: {e}"))?;
    }
    Ok(())
}

/// Create a logical document or return its original receipt.
pub fn add(root: &Path, embedder: &dyn Embedder, input: Input) -> anyhow::Result<Receipt> {
    add_inner(root, embedder, input, None)
}

fn add_inner(
    root: &Path,
    embedder: &dyn Embedder,
    mut input: Input,
    image: Option<(ImageAsset, Vec<u8>, Value)>,
) -> anyhow::Result<Receipt> {
    validate(&input)?;
    input.tags.sort();
    input.tags.dedup();
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    let root = &lock.path;
    recover_locked(root)?;
    let repo = RedbDocumentRepository::open(&path(root, "documents.redb"))?;
    let fingerprint = if let Some((asset, _, details)) = &image {
        content_hash(&serde_json::to_vec(&(&input, asset, details))?)
    } else {
        content_hash(&serde_json::to_vec(&input)?)
    };
    if let Some(existing) = read_record(&repo, &input.source_key)? {
        if existing.fingerprint != fingerprint {
            anyhow::bail!("source_conflict");
        }
        let doc = repo.find_by_path(&existing.receipt.source_uri)?;
        if doc
            .as_ref()
            .is_none_or(|doc| doc.status == DocumentStatus::Deleted)
        {
            anyhow::bail!("source_conflict");
        }
        let mut receipt = existing.receipt;
        receipt.status = "unchanged".into();
        return Ok(receipt);
    }
    let uri = format!("mcp://local/{}", input.source_key);
    if repo.find_by_path(&uri)?.is_some() {
        anyhow::bail!("source_conflict");
    }
    let model = embedder.model_version();
    if let Some(indexed) = repo.indexed_model()? {
        if indexed != (model.name.clone(), model.dimension) {
            anyhow::bail!("embedding_model_mismatch");
        }
    }
    let ext = if input.format == "markdown" {
        "md"
    } else {
        "txt"
    };
    let parsers = default_parsers();
    let parser = parsers
        .iter()
        .find(|p| p.extensions().contains(&ext))
        .unwrap();
    let mut parsed = parser.parse(input.content.as_bytes())?;
    if input.title.is_some() {
        parsed.title = input.title.clone();
    }
    let did = document_id(&uri);
    let mut extra = std::collections::BTreeMap::new();
    if !input.metadata.is_empty() {
        extra.insert(
            "user_metadata".into(),
            serde_json::to_string(&input.metadata)?,
        );
    }
    if let Some((asset, _, _)) = &image {
        extra.insert("source_kind".into(), "image".into());
        extra.insert("asset_id".into(), asset.asset_id.clone());
    }
    let (chunks, vectors) = prepare_chunks(
        &ChunkingService::new(ChunkingConfig::default()),
        &parsed,
        ext,
        &did,
        &uri,
        embedder,
        &extra,
    )?;
    if chunks.is_empty() {
        anyhow::bail!("invalid_input");
    }
    let timestamp = now();
    let receipt = Receipt {
        status: "created".into(),
        document_id: did.clone(),
        source_uri: uri.clone(),
        content_hash: content_hash(input.content.as_bytes()),
        chunk_count: chunks.len(),
        embedding_model: model.name.clone(),
        embedding_dimension: model.dimension,
        indexed_at: timestamp.clone(),
    };
    let doc = IngestionDocument {
        id: did,
        source_path: uri,
        file_name: format!("{}.{}", input.source_key, ext),
        file_extension: ext.into(),
        content_hash: receipt.content_hash.clone(),
        title: parsed.title,
        project: input.project.clone(),
        tags: input.tags.clone(),
        status: DocumentStatus::Active,
        embedding_model: model.name.clone(),
        embedding_dimension: model.dimension,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        deleted_at: None,
    };
    let (image_asset, image_stage, image_details) = match image {
        Some((asset, bytes, details)) => {
            let stage = assets::stage(root, &bytes)?;
            (Some(asset), Some(stage), Some(details))
        }
        None => (None, None, None),
    };
    let pending = Pending {
        stored: Stored {
            fingerprint,
            receipt: receipt.clone(),
            source_key: input.source_key,
            metadata: input.metadata,
            preview: chunks[0].preview.clone(),
            image: image_asset,
            image_details,
        },
        document: doc,
        chunks,
        vectors,
        chunking: ChunkingConfig::default(),
        image_bytes: None,
        image_stage,
    };
    if let Err(error) = save_pending(&repo, &pending) {
        if let Some(stage) = &pending.image_stage {
            let _ = assets::discard_staged(root, stage);
        }
        return Err(error);
    }
    apply(root, &repo, &pending)?;
    Ok(receipt)
}

/// Preserve an image and index its supplied description and optional OCR.
pub fn add_image(
    root: &Path,
    embedder: &dyn Embedder,
    mut input: ImageInput,
    bytes: Vec<u8>,
) -> anyhow::Result<Value> {
    if input.description.trim().is_empty() {
        anyhow::bail!("invalid_input");
    }
    input.tags.sort();
    input.tags.dedup();
    let asset = assets::validate(&bytes, &input.mime_type)?;
    let details = serde_json::to_value(&input)?;
    let mut content = String::new();
    if let Some(title) = &input.title {
        content.push_str(&format!("Title: {title}\n"));
    }
    content.push_str(&format!("Description: {}\n", input.description));
    if let Some(ocr) = &input.ocr_text {
        content.push_str(&format!("Extracted text: {ocr}\n"));
    }
    if let Some(context) = &input.context {
        content.push_str(&format!("Context: {context}\n"));
    }
    let document = Input {
        source_key: input.source_key,
        content,
        format: "text".into(),
        title: input.title,
        project: input.project,
        tags: input.tags,
        metadata: Map::new(),
    };
    let receipt = add_inner(
        root,
        embedder,
        document,
        Some((asset.clone(), bytes, details)),
    )?;
    let mut value = serde_json::to_value(receipt)?;
    value["asset_id"] = json!(asset.asset_id);
    value["original_hash"] = json!(asset.content_hash);
    value["mime_type"] = json!(asset.mime_type);
    value["byte_size"] = json!(asset.byte_size);
    Ok(value)
}

/// Fetch an image asset from its committed asset record.
pub fn get_image(root: &Path, asset_id: &str) -> anyhow::Result<(ImageAsset, Vec<u8>)> {
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    recover_locked(&lock.path)?;
    let repo = RedbDocumentRepository::open(&path(&lock.path, "documents.redb"))?;
    let asset: ImageAsset = {
        let tx = repo.db.begin_read()?;
        let table = match tx.open_table(ASSETS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => anyhow::bail!("asset_not_found"),
            Err(error) => return Err(error.into()),
        };
        let value = table
            .get(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("asset_not_found"))?;
        serde_json::from_str(value.value())?
    };
    let bytes = assets::read(&lock.path, &asset)?;
    Ok((asset, bytes))
}

/// Find an active, fully committed logical document by its source key.
pub fn query(root: &Path, source_key: &str) -> anyhow::Result<Option<Value>> {
    let lock = StoreLock::acquire(root, Duration::from_secs(5))?;
    recover_locked(&lock.path)?;
    let repo = RedbDocumentRepository::open(&path(&lock.path, "documents.redb"))?;
    let Some(record) = read_record(&repo, source_key)? else {
        return Ok(None);
    };
    let Some(doc) = repo.find_by_path(&record.receipt.source_uri)? else {
        return Ok(None);
    };
    if doc.status == DocumentStatus::Deleted {
        return Ok(None);
    }
    let mut value = serde_json::to_value(record.receipt)?;
    let obj = value.as_object_mut().unwrap();
    obj.remove("status");
    obj.insert("source_key".into(), json!(source_key));
    obj.insert(
        "source_kind".into(),
        json!(if record.image.is_some() {
            "image"
        } else {
            "text"
        }),
    );
    if let Some(asset) = record.image {
        obj.insert("asset_id".into(), json!(asset.asset_id));
        obj.insert("original_hash".into(), json!(asset.content_hash));
        obj.insert("mime_type".into(), json!(asset.mime_type));
        obj.insert("byte_size".into(), json!(asset.byte_size));
    }
    obj.insert("title".into(), json!(doc.title));
    obj.insert("project".into(), json!(doc.project));
    obj.insert("tags".into(), json!(doc.tags));
    obj.insert("metadata".into(), json!(record.metadata));
    obj.insert("preview".into(), json!(record.preview));
    Ok(Some(value))
}
