use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tovli::ingestion::durable::{add, query, Input};
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::store_lock::StoreLock;

fn root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("tovli-durable-{}-{nanos}", std::process::id()))
}

fn input() -> Input {
    serde_json::from_value(serde_json::json!({
        "source_key": "payment-note",
        "content": "# Retry\nA failed job is retried.",
        "project": "payments",
        "metadata": {"reviewed": true, "severity": 2}
    }))
    .unwrap()
}

#[test]
fn creates_queries_and_retries_without_duplicates() {
    let root = root();
    let embedder = MockEmbedder::new(8);
    let first = add(&root, &embedder, input()).unwrap();
    assert_eq!(first.status, "created");
    let result = query(&root, "payment-note").unwrap().unwrap();
    assert_eq!(result["metadata"]["reviewed"], true);
    assert_eq!(result["metadata"]["severity"], 2);
    assert_eq!(result["document_id"], first.document_id);
    let second = add(&root, &embedder, input()).unwrap();
    assert_eq!(second.status, "unchanged");
    assert_eq!(second.document_id, first.document_id);
    let mut conflict = input();
    conflict
        .metadata
        .insert("severity".into(), serde_json::json!(3));
    assert!(add(&root, &embedder, conflict)
        .unwrap_err()
        .to_string()
        .contains("source_conflict"));
    assert_eq!(query(&root, "missing").unwrap(), None);
    {
        use tovli::ingestion::ports::Embedder;
        use tovli::retrieval::application::SearchService;
        use tovli::retrieval::domain::{MetadataFilter, Query, SearchMode};
        use tovli::retrieval::infra::redb_keyword_search::RedbKeywordSearch;
        use tovli::retrieval::infra::redb_lookup::RedbDocumentLookup;
        use tovli::retrieval::infra::ruvector_search::RuVectorSearchAdapter;

        let vector_path = root.join("vectors.redb").to_string_lossy().to_string();
        let docs_path = root.join("documents.redb").to_string_lossy().to_string();
        let keyword_path = root.join("keyword.redb").to_string_lossy().to_string();
        let store = RuVectorSearchAdapter::open(&vector_path, 8).unwrap();
        let lookup = RedbDocumentLookup::open(&docs_path).unwrap();
        let keyword = RedbKeywordSearch::open(&keyword_path).unwrap();
        let service = SearchService {
            embedder: &embedder,
            store: &store,
            keyword: &keyword,
            lookup: &lookup,
        };
        for mode in [SearchMode::Vector, SearchMode::Keyword, SearchMode::Hybrid] {
            let query = Query {
                text: "retried".into(),
                mode,
                filters: MetadataFilter::default(),
                top_k: 8,
                embedding_model: embedder.model_version().clone(),
            };
            let run = service.search(&query, false, "run", "2026-01-01").unwrap();
            assert!(
                run.results
                    .iter()
                    .any(|result| result.document_id == first.document_id),
                "{mode:?} did not find the saved document: {:?}",
                run.results,
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn store_lock_is_released_after_guard_drops() {
    let root = root();
    let guard = StoreLock::acquire(&root, Duration::from_millis(50)).unwrap();
    let error = match StoreLock::acquire(&root, Duration::from_millis(100)) {
        Ok(_) => panic!("second lock unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("store_in_use"), "{error}");
    drop(guard);
    drop(StoreLock::acquire(&root, Duration::from_millis(50)).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_index_write_is_recovered_before_query() {
    use redb::{Database, ReadableTable, TableDefinition};
    use tovli::ingestion::ports::Embedder;
    use tovli::lexical_index::{KeywordIndexedChunk, KEYWORD_CHUNKS};
    const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");
    const RECEIPTS: TableDefinition<&str, &str> = TableDefinition::new("mcp_receipts");
    const JOURNAL: TableDefinition<&str, &str> = TableDefinition::new("mcp_journal");

    for stage in 0..4 {
        let root = root();
        let embedder = MockEmbedder::new(8);
        let receipt = add(&root, &embedder, input()).unwrap();
        let keywords = Database::open(root.join("keyword.redb")).unwrap();
        let indexed: KeywordIndexedChunk = {
            let tx = keywords.begin_read().unwrap();
            let table = tx.open_table(KEYWORD_CHUNKS).unwrap();
            let (_, value) = table.iter().unwrap().next().unwrap().unwrap();
            serde_json::from_str(value.value()).unwrap()
        };
        drop(keywords);
        let vector = embedder.embed_batch(&[&indexed.content]).unwrap().remove(0);
        let chunk = serde_json::json!({
            "id": indexed.chunk_id,
            "document_id": indexed.document_id,
            "chunk_index": 0,
            "heading_path": indexed.heading_path,
            "content": indexed.content,
            "preview": indexed.preview,
            "content_hash": "test",
            "token_count": 3,
            "metadata": indexed.metadata,
        });
        if stage == 0 {
            let store = tovli::ingestion::infra::ruvector_store::RuVectorStoreAdapter::open(
                &root.join("vectors.redb").to_string_lossy(),
                &root.join("chunkmap.redb").to_string_lossy(),
                8,
            )
            .unwrap();
            store
                .delete_chunks(std::slice::from_ref(&indexed.chunk_id))
                .unwrap();
        }
        if stage <= 1 {
            let db = Database::open(root.join("chunkmap.redb")).unwrap();
            let tx = db.begin_write().unwrap();
            const DOC_CHUNKS: TableDefinition<&str, &str> = TableDefinition::new("doc_chunks");
            tx.open_table(DOC_CHUNKS)
                .unwrap()
                .remove(indexed.document_id.as_str())
                .unwrap();
            tx.commit().unwrap();
        }
        if stage <= 2 {
            let db = Database::open(root.join("keyword.redb")).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(KEYWORD_CHUNKS)
                .unwrap()
                .remove(indexed.chunk_id.as_str())
                .unwrap();
            tx.commit().unwrap();
        }
        let db = Database::open(root.join("documents.redb")).unwrap();
        let (document, stored) = {
            let tx = db.begin_read().unwrap();
            let docs = tx.open_table(DOCS).unwrap();
            let receipts = tx.open_table(RECEIPTS).unwrap();
            let document: serde_json::Value = serde_json::from_str(
                docs.get(receipt.source_uri.as_str())
                    .unwrap()
                    .unwrap()
                    .value(),
            )
            .unwrap();
            let stored: serde_json::Value =
                serde_json::from_str(receipts.get("payment-note").unwrap().unwrap().value())
                    .unwrap();
            (document, stored)
        };
        let pending = serde_json::json!({"stored": stored, "document": document, "chunks": [chunk], "vectors": [vector]});
        let tx = db.begin_write().unwrap();
        {
            tx.open_table(DOCS)
                .unwrap()
                .remove(receipt.source_uri.as_str())
                .unwrap();
            tx.open_table(RECEIPTS)
                .unwrap()
                .remove("payment-note")
                .unwrap();
            tx.open_table(JOURNAL)
                .unwrap()
                .insert(
                    "payment-note",
                    serde_json::to_string(&pending).unwrap().as_str(),
                )
                .unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let recovered = query(&root, "payment-note").unwrap().unwrap();
        assert_eq!(recovered["document_id"], receipt.document_id);
        assert_eq!(add(&root, &embedder, input()).unwrap().status, "unchanged");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn tombstoned_memory_is_not_returned_or_recreated() {
    use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
    use tovli::ingestion::ports::DocumentRepository;

    let root = root();
    let embedder = MockEmbedder::new(8);
    let receipt = add(&root, &embedder, input()).unwrap();
    {
        let docs =
            RedbDocumentRepository::open(&root.join("documents.redb").to_string_lossy()).unwrap();
        docs.soft_delete(&receipt.document_id).unwrap();
    }
    assert!(query(&root, "payment-note").unwrap().is_none());
    assert!(add(&root, &embedder, input())
        .unwrap_err()
        .to_string()
        .contains("source_conflict"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn incompatible_embedding_model_cannot_modify_existing_store() {
    let root = root();
    let embedder = MockEmbedder::new(8);
    let first = add(&root, &embedder, input()).unwrap();
    let mut another = input();
    another.source_key = "different-key".into();
    let error = add(&root, &MockEmbedder::new(16), another).unwrap_err();
    assert!(error.to_string().contains("embedding_model_mismatch"));
    assert!(query(&root, "different-key").unwrap().is_none());
    assert_eq!(
        query(&root, "payment-note").unwrap().unwrap()["document_id"],
        first.document_id
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupted_journal_blocks_reads_and_writes() {
    use redb::{Database, TableDefinition};
    const JOURNAL: TableDefinition<&str, &str> = TableDefinition::new("mcp_journal");

    let root = root();
    std::fs::create_dir_all(&root).unwrap();
    {
        let db = Database::create(root.join("documents.redb")).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(JOURNAL).unwrap().insert("bad", "{").unwrap();
        tx.commit().unwrap();
    }
    assert!(query(&root, "anything")
        .unwrap_err()
        .to_string()
        .contains("recovery_required"));
    assert!(add(&root, &MockEmbedder::new(8), input())
        .unwrap_err()
        .to_string()
        .contains("recovery_required"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_rejects_journal_with_different_indexed_model() {
    use redb::{Database, ReadableTable, TableDefinition};
    const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");
    const RECEIPTS: TableDefinition<&str, &str> = TableDefinition::new("mcp_receipts");
    const JOURNAL: TableDefinition<&str, &str> = TableDefinition::new("mcp_journal");
    let root = root();
    let receipt = add(&root, &MockEmbedder::new(8), input()).unwrap();
    {
        let db = Database::open(root.join("documents.redb")).unwrap();
        let (mut doc, stored): (serde_json::Value, serde_json::Value) = {
            let tx = db.begin_read().unwrap();
            let docs = tx.open_table(DOCS).unwrap();
            let receipts = tx.open_table(RECEIPTS).unwrap();
            let doc = serde_json::from_str(
                docs.get(receipt.source_uri.as_str())
                    .unwrap()
                    .unwrap()
                    .value(),
            )
            .unwrap();
            let stored =
                serde_json::from_str(receipts.get("payment-note").unwrap().unwrap().value())
                    .unwrap();
            (doc, stored)
        };
        doc["embedding_model"] = serde_json::json!("different-model");
        let pending =
            serde_json::json!({"stored": stored, "document": doc, "chunks": [], "vectors": []});
        let tx = db.begin_write().unwrap();
        tx.open_table(JOURNAL)
            .unwrap()
            .insert("pending", serde_json::to_string(&pending).unwrap().as_str())
            .unwrap();
        tx.commit().unwrap();
    }
    let error = query(&root, "payment-note").unwrap_err().to_string();
    assert!(error.contains("recovery_required"));
    assert!(error.contains("indexed embedding model mismatch"));
    {
        let db = Database::open(root.join("documents.redb")).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(JOURNAL).unwrap();
            let mut pending: serde_json::Value =
                serde_json::from_str(table.get("pending").unwrap().unwrap().value()).unwrap();
            pending["document"]["embedding_model"] = serde_json::json!("mock-deterministic");
            table
                .insert("pending", serde_json::to_string(&pending).unwrap().as_str())
                .unwrap();
        }
        tx.commit().unwrap();
    }
    let error = query(&root, "payment-note").unwrap_err().to_string();
    assert!(error.contains("invalid pending operation"));
    {
        let db = Database::open(root.join("documents.redb")).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(JOURNAL).unwrap().remove("pending").unwrap();
        tx.commit().unwrap();
    }
    assert!(query(&root, "payment-note").unwrap().is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_reconciliation_does_not_match_sibling_prefixes_or_logical_sources() {
    use tovli::ingestion::infra::redb_repo::RedbDocumentRepository;
    use tovli::ingestion::ports::DocumentRepository;

    let root = root();
    let receipt = add(&root, &MockEmbedder::new(8), input()).unwrap();
    {
        let docs =
            RedbDocumentRepository::open(&root.join("documents.redb").to_string_lossy()).unwrap();
        let template = docs.find_by_path(&receipt.source_uri).unwrap().unwrap();
        let folder = root.join("notes");
        let wanted = folder.join("wanted.md").to_string_lossy().to_string();
        let sibling = root
            .join("notes-old")
            .join("unrelated.md")
            .to_string_lossy()
            .to_string();
        let mut file = template.clone();
        file.id = "file-wanted".into();
        file.source_path = wanted.clone();
        docs.save(&file).unwrap();
        file.id = "file-sibling".into();
        file.source_path = sibling;
        docs.save(&file).unwrap();
        let matches = docs.active_under(&folder.to_string_lossy()).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source_path, wanted);
        assert!(docs.active_under("mcp://").unwrap().is_empty());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn image_recovery_publishes_staged_or_previously_published_original() {
    use image::{DynamicImage, ImageOutputFormat, RgbaImage};
    use redb::{Database, ReadableTable, TableDefinition};
    use std::io::Cursor;
    use tovli::image_memory::assets;
    use tovli::ingestion::durable::{add_image, get_image, ImageInput};
    use tovli::ingestion::ports::Embedder;
    use tovli::lexical_index::{KeywordIndexedChunk, KEYWORD_CHUNKS};
    const DOCS: TableDefinition<&str, &str> = TableDefinition::new("documents");
    const RECEIPTS: TableDefinition<&str, &str> = TableDefinition::new("mcp_receipts");
    const JOURNAL: TableDefinition<&str, &str> = TableDefinition::new("mcp_journal");
    const ASSETS: TableDefinition<&str, &str> = TableDefinition::new("image_assets");

    for published in [false, true] {
        let root = root();
        let embedder = MockEmbedder::new(8);
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(2, 3))
            .write_to(&mut png, ImageOutputFormat::Png)
            .unwrap();
        let bytes = png.into_inner();
        let image_input: ImageInput = serde_json::from_value(serde_json::json!({
            "source_key": "screenshot",
            "mime_type": "image/png",
            "description": "Queue diagram"
        }))
        .unwrap();
        let receipt = add_image(&root, &embedder, image_input, bytes.clone()).unwrap();
        let asset = assets::validate(&bytes, "image/png").unwrap();
        let indexed: KeywordIndexedChunk = {
            let db = Database::open(root.join("keyword.redb")).unwrap();
            let tx = db.begin_read().unwrap();
            let table = tx.open_table(KEYWORD_CHUNKS).unwrap();
            let (_, value) = table.iter().unwrap().next().unwrap().unwrap();
            serde_json::from_str(value.value()).unwrap()
        };
        let vector = embedder.embed_batch(&[&indexed.content]).unwrap().remove(0);
        let chunk = serde_json::json!({
            "id": indexed.chunk_id, "document_id": indexed.document_id, "chunk_index": 0,
            "heading_path": indexed.heading_path, "content": indexed.content,
            "preview": indexed.preview, "content_hash": "test", "token_count": 3,
            "metadata": indexed.metadata
        });
        std::fs::remove_file(
            root.join("assets")
                .join(format!("{}.png", asset.content_hash)),
        )
        .unwrap();
        let stage = assets::stage(&root, &bytes).unwrap();
        if published {
            assets::publish_staged(&root, &asset, &stage).unwrap();
        }
        let db = Database::open(root.join("documents.redb")).unwrap();
        let source = receipt["source_uri"].as_str().unwrap();
        let (document, stored) = {
            let tx = db.begin_read().unwrap();
            let docs = tx.open_table(DOCS).unwrap();
            let receipts = tx.open_table(RECEIPTS).unwrap();
            let document: serde_json::Value =
                serde_json::from_str(docs.get(source).unwrap().unwrap().value()).unwrap();
            let stored: serde_json::Value =
                serde_json::from_str(receipts.get("screenshot").unwrap().unwrap().value()).unwrap();
            (document, stored)
        };
        let pending = serde_json::json!({
            "stored": stored, "document": document, "chunks": [chunk], "vectors": [vector],
            "image_stage": stage
        });
        let tx = db.begin_write().unwrap();
        {
            tx.open_table(DOCS).unwrap().remove(source).unwrap();
            tx.open_table(RECEIPTS)
                .unwrap()
                .remove("screenshot")
                .unwrap();
            tx.open_table(ASSETS)
                .unwrap()
                .remove(asset.asset_id.as_str())
                .unwrap();
            tx.open_table(JOURNAL)
                .unwrap()
                .insert(
                    "screenshot",
                    serde_json::to_string(&pending).unwrap().as_str(),
                )
                .unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let found = query(&root, "screenshot").unwrap().unwrap();
        assert_eq!(found["asset_id"], asset.asset_id);
        assert_eq!(get_image(&root, &asset.asset_id).unwrap().1, bytes);
        std::fs::remove_dir_all(root).unwrap();
    }
}
