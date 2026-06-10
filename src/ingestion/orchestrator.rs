//! IngestionOrchestrator — coordinates the full pipeline for one `tovli ingest` run:
//! scan → parse → dedup(hash) → chunk → embed → persist → detect deletions.
//! Generic over the ports, so it is fully unit-testable with in-memory fakes.

use std::collections::HashSet;
use std::path::Path;

use walkdir::WalkDir;

use crate::ingestion::chunking::ChunkingService;
use crate::ingestion::domain::{
    content_hash, document_id, DocumentId, DocumentStatus, IngestionDocument, IngestionSummary,
};
use crate::ingestion::embedding::EmbeddingService;
use crate::ingestion::ports::{
    ChunkWithEmbedding, DocumentRepository, Embedder, FileParser, KeywordIndexPort, VectorStorePort,
};

#[derive(Debug, Clone, Default)]
pub struct IngestOptions {
    pub dry_run: bool,
    pub force: bool,
    pub project: Option<String>,
    pub tags: Vec<String>,
}

pub struct IngestionOrchestrator<'a> {
    pub parsers: &'a [Box<dyn FileParser>],
    pub embedder: &'a dyn Embedder,
    pub store: &'a dyn VectorStorePort,
    pub keyword_index: Option<&'a dyn KeywordIndexPort>,
    pub docs: &'a dyn DocumentRepository,
    pub chunking: ChunkingService,
}

impl<'a> IngestionOrchestrator<'a> {
    fn parser_for(&self, ext: &str) -> Option<&dyn FileParser> {
        self.parsers
            .iter()
            .find(|p| p.extensions().iter().any(|x| x.eq_ignore_ascii_case(ext)))
            .map(|b| b.as_ref())
    }

    /// Run ingestion over `root`. `now` is an injected timestamp (RFC3339) for determinism.
    pub fn ingest(
        &self,
        root: &Path,
        opts: &IngestOptions,
        now: &str,
    ) -> anyhow::Result<IngestionSummary> {
        let mut summary = IngestionSummary {
            dry_run: opts.dry_run,
            ..Default::default()
        };
        let embed = EmbeddingService::new(self.embedder);
        let model = embed.model_version().clone();
        let root_str = root.to_string_lossy().to_string();
        let mut seen: HashSet<DocumentId> = HashSet::new();

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            summary.files_scanned += 1;
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let Some(parser) = self.parser_for(&ext) else {
                summary.files_skipped += 1;
                summary
                    .skipped
                    .push((path_str, format!("unsupported extension '.{ext}'")));
                continue;
            };

            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    summary.files_errored += 1;
                    summary.errors.push((path_str, format!("read error: {e}")));
                    continue;
                }
            };
            let fhash = content_hash(&bytes);
            let did = document_id(&path_str);
            let existing = self.docs.find_by_path(&path_str)?;

            // Idempotency: unchanged hash and not forced → skip (AC-3).
            if let Some(ref doc) = existing {
                if doc.content_hash == fhash && !opts.force {
                    summary.files_unchanged += 1;
                    seen.insert(doc.id.clone());
                    continue;
                }
            }

            let parsed = match parser.parse(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    summary.files_skipped += 1;
                    summary.skipped.push((path_str, e.to_string()));
                    continue;
                }
            };

            let mut chunks = self.chunking.chunk(&parsed, &ext, &did);
            // Enrich chunk metadata (R5 / AC-6).
            for c in &mut chunks {
                let char_len = c.content.chars().count();
                c.metadata.insert("source_path".into(), path_str.clone());
                c.metadata
                    .insert("char_length".into(), char_len.to_string());
                c.metadata
                    .insert("embedding_model".into(), model.name.clone());
                c.metadata
                    .insert("embedding_dimension".into(), model.dimension.to_string());
                if let Some(t) = &parsed.title {
                    c.metadata.insert("title".into(), t.clone());
                }
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let mut doc = IngestionDocument {
                id: did.clone(),
                source_path: path_str.clone(),
                file_name,
                file_extension: ext.clone(),
                content_hash: fhash,
                title: parsed.title.clone(),
                project: opts.project.clone(),
                tags: opts.tags.clone(),
                status: if existing.is_some() {
                    DocumentStatus::Modified
                } else {
                    DocumentStatus::Active
                },
                embedding_model: model.name.clone(),
                embedding_dimension: model.dimension,
                created_at: existing
                    .as_ref()
                    .map(|d| d.created_at.clone())
                    .unwrap_or_else(|| now.to_string()),
                updated_at: now.to_string(),
                deleted_at: None,
            };

            if chunks.is_empty() {
                // edge case E1
                summary.files_empty += 1;
                if !opts.dry_run {
                    if existing.is_some() {
                        self.store.delete_by_document(&did)?;
                        if let Some(keyword_index) = self.keyword_index {
                            keyword_index.delete_by_document(&did)?;
                        }
                    }
                    self.docs.save(&doc)?;
                }
                seen.insert(did);
                continue;
            }

            let vectors = embed.embed_chunks(&chunks)?;

            if opts.dry_run {
                // AC-7: report, no writes.
                summary.files_ingested += 1;
                summary.chunks_created += chunks.len();
                seen.insert(did);
                continue;
            }

            // Modified file → drop old chunks before re-indexing (AC-4).
            if existing.is_some() {
                self.store.delete_by_document(&did)?;
                if let Some(keyword_index) = self.keyword_index {
                    keyword_index.delete_by_document(&did)?;
                }
            }
            let items: Vec<ChunkWithEmbedding> = chunks
                .iter()
                .zip(vectors)
                .map(|(c, v)| ChunkWithEmbedding {
                    chunk: c,
                    vector: v,
                })
                .collect();
            self.store.upsert_chunks(&items)?; // AC-8 dim guard lives in the adapter
            if let Some(keyword_index) = self.keyword_index {
                let chunk_refs: Vec<_> = chunks.iter().collect();
                keyword_index.upsert_chunks(&chunk_refs)?;
            }

            doc.status = DocumentStatus::Active;
            self.docs.save(&doc)?;
            summary.files_ingested += 1;
            summary.chunks_created += chunks.len();
            seen.insert(did);
        }

        // Deletion pass: active docs under root not seen this run (R4 / E5).
        if !opts.dry_run {
            for doc in self.docs.active_under(&root_str)? {
                if !seen.contains(&doc.id) {
                    self.docs.soft_delete(&doc.id)?;
                    self.store.delete_by_document(&doc.id)?;
                    if let Some(keyword_index) = self.keyword_index {
                        keyword_index.delete_by_document(&doc.id)?;
                    }
                    summary.files_deleted += 1;
                }
            }
        }

        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::domain::ChunkingConfig;
    use crate::ingestion::infra::mock_embedder::MockEmbedder;
    use crate::ingestion::infra::parsers::default_parsers;
    use crate::ingestion::ports::KeywordIndexPort;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---- in-memory fakes ----
    #[derive(Default)]
    struct MemRepo {
        by_path: RefCell<HashMap<String, IngestionDocument>>,
    }
    impl DocumentRepository for MemRepo {
        fn find_by_path(&self, path: &str) -> anyhow::Result<Option<IngestionDocument>> {
            Ok(self.by_path.borrow().get(path).cloned())
        }
        fn save(&self, doc: &IngestionDocument) -> anyhow::Result<()> {
            self.by_path
                .borrow_mut()
                .insert(doc.source_path.clone(), doc.clone());
            Ok(())
        }
        fn soft_delete(&self, id: &DocumentId) -> anyhow::Result<()> {
            for d in self.by_path.borrow_mut().values_mut() {
                if &d.id == id {
                    d.status = DocumentStatus::Deleted;
                }
            }
            Ok(())
        }
        fn active_under(&self, root: &str) -> anyhow::Result<Vec<IngestionDocument>> {
            Ok(self
                .by_path
                .borrow()
                .values()
                .filter(|d| d.status != DocumentStatus::Deleted && d.source_path.starts_with(root))
                .cloned()
                .collect())
        }
    }

    struct MemStore {
        dim: usize,
        by_doc: RefCell<HashMap<DocumentId, Vec<String>>>,
    }
    impl MemStore {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                by_doc: RefCell::new(HashMap::new()),
            }
        }
        fn chunk_count(&self) -> usize {
            self.by_doc.borrow().values().map(|v| v.len()).sum()
        }
    }
    impl VectorStorePort for MemStore {
        fn upsert_chunks(&self, items: &[ChunkWithEmbedding]) -> anyhow::Result<()> {
            for it in items {
                if it.vector.len() != self.dim {
                    anyhow::bail!(
                        "dim mismatch: vector {} != index {}",
                        it.vector.len(),
                        self.dim
                    );
                }
                self.by_doc
                    .borrow_mut()
                    .entry(it.chunk.document_id.clone())
                    .or_default()
                    .push(it.chunk.id.clone());
            }
            Ok(())
        }
        fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()> {
            self.by_doc.borrow_mut().remove(id);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemKeywordIndex {
        by_doc: RefCell<HashMap<DocumentId, Vec<String>>>,
        contents: RefCell<HashMap<String, String>>,
    }
    impl KeywordIndexPort for MemKeywordIndex {
        fn upsert_chunks(&self, chunks: &[&crate::ingestion::domain::Chunk]) -> anyhow::Result<()> {
            for chunk in chunks {
                self.by_doc
                    .borrow_mut()
                    .entry(chunk.document_id.clone())
                    .or_default()
                    .push(chunk.id.clone());
                self.contents
                    .borrow_mut()
                    .insert(chunk.id.clone(), chunk.content.clone());
            }
            Ok(())
        }

        fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()> {
            let removed = self.by_doc.borrow_mut().remove(id).unwrap_or_default();
            for chunk_id in removed {
                self.contents.borrow_mut().remove(&chunk_id);
            }
            Ok(())
        }
    }

    // ---- helpers ----
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    fn tmpdir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("tovli-it-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }
    const DIM: usize = 8;
    const NOW: &str = "2026-06-02T00:00:00Z";

    fn orchestrator<'a>(
        parsers: &'a [Box<dyn FileParser>],
        embedder: &'a MockEmbedder,
        store: &'a MemStore,
        docs: &'a MemRepo,
    ) -> IngestionOrchestrator<'a> {
        IngestionOrchestrator {
            parsers,
            embedder,
            store,
            keyword_index: None,
            docs,
            chunking: ChunkingService::new(ChunkingConfig::default()),
        }
    }

    #[test]
    fn ingests_and_reports_summary_with_metadata() {
        // AC-1 + AC-6
        let dir = tmpdir();
        write(&dir, "a.md", "# Arch\n\nlayering rules and boundaries\n");
        write(&dir, "b.txt", "plain text content here");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
        );
        let o = orchestrator(&parsers, &emb, &store, &docs);

        let s = o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert_eq!(s.files_scanned, 2);
        assert_eq!(s.files_ingested, 2);
        assert!(s.chunks_created >= 2);
        assert_eq!(store.chunk_count(), s.chunks_created);
        // AC-6: metadata persisted on the document record
        let saved = docs
            .find_by_path(&dir.join("a.md").to_string_lossy())
            .unwrap()
            .unwrap();
        assert_eq!(saved.embedding_dimension, DIM);
        assert_eq!(saved.embedding_model, "mock-deterministic");
        assert_eq!(saved.title.as_deref(), Some("Arch"));
    }

    #[test]
    fn skips_unsupported_files() {
        // AC-2
        let dir = tmpdir();
        write(&dir, "keep.md", "# K\n\nbody");
        write(&dir, "image.png", "not really an image");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
        );
        let s = orchestrator(&parsers, &emb, &store, &docs)
            .ingest(&dir, &IngestOptions::default(), NOW)
            .unwrap();
        assert_eq!(s.files_ingested, 1);
        assert_eq!(s.files_skipped, 1);
        assert!(s
            .skipped
            .iter()
            .any(|(p, r)| p.ends_with("image.png") && r.contains("unsupported")));
    }

    #[test]
    fn is_idempotent_on_second_run() {
        // AC-3
        let dir = tmpdir();
        write(&dir, "a.md", "# A\n\ncontent one two three");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
        );
        let o = orchestrator(&parsers, &emb, &store, &docs);
        let first = o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert_eq!(first.files_ingested, 1);
        let second = o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert_eq!(second.files_unchanged, 1);
        assert_eq!(second.files_ingested, 0);
        assert_eq!(second.chunks_created, 0);
    }

    #[test]
    fn rechunks_only_modified_file() {
        // AC-4
        let dir = tmpdir();
        write(&dir, "a.md", "# A\n\noriginal content");
        write(&dir, "b.md", "# B\n\nstable content");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
        );
        let o = orchestrator(&parsers, &emb, &store, &docs);
        o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        write(&dir, "a.md", "# A\n\nCHANGED content entirely now");
        let s = o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert_eq!(s.files_ingested, 1, "only the modified file re-ingests");
        assert_eq!(s.files_unchanged, 1, "the stable file is skipped");
    }

    #[test]
    fn syncs_keyword_index_on_insert_modify_and_delete() {
        let dir = tmpdir();
        write(&dir, "a.md", "# A\n\nfirst lexical content");
        let (parsers, emb, store, docs, keyword) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
            MemKeywordIndex::default(),
        );
        let o = IngestionOrchestrator {
            parsers: &parsers,
            embedder: &emb,
            store: &store,
            keyword_index: Some(&keyword),
            docs: &docs,
            chunking: ChunkingService::new(ChunkingConfig::default()),
        };

        o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert!(keyword
            .contents
            .borrow()
            .values()
            .any(|c| c.contains("first lexical content")));

        write(&dir, "a.md", "# A\n\nsecond lexical content");
        o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        let indexed_contents: Vec<String> = keyword.contents.borrow().values().cloned().collect();
        assert!(indexed_contents
            .iter()
            .any(|c| c.contains("second lexical content")));
        assert!(!indexed_contents
            .iter()
            .any(|c| c.contains("first lexical content")));

        std::fs::remove_file(dir.join("a.md")).unwrap();
        o.ingest(&dir, &IngestOptions::default(), NOW).unwrap();
        assert!(
            keyword.contents.borrow().is_empty(),
            "deleted documents should remove keyword chunks"
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        // AC-7
        let dir = tmpdir();
        write(&dir, "a.md", "# A\n\nbody content here");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(DIM),
            MemStore::new(DIM),
            MemRepo::default(),
        );
        let opts = IngestOptions {
            dry_run: true,
            ..Default::default()
        };
        let s = orchestrator(&parsers, &emb, &store, &docs)
            .ingest(&dir, &opts, NOW)
            .unwrap();
        assert!(s.dry_run);
        assert_eq!(s.files_ingested, 1);
        assert!(s.chunks_created >= 1);
        assert_eq!(store.chunk_count(), 0, "no vectors written");
        assert!(
            docs.find_by_path(&dir.join("a.md").to_string_lossy())
                .unwrap()
                .is_none(),
            "no doc record written"
        );
    }

    #[test]
    fn rejects_dimension_mismatch() {
        // AC-8: embedder dim != index dim → error
        let dir = tmpdir();
        write(&dir, "a.md", "# A\n\nbody");
        let (parsers, emb, store, docs) = (
            default_parsers(),
            MockEmbedder::new(4),
            MemStore::new(8),
            MemRepo::default(),
        );
        let r = orchestrator(&parsers, &emb, &store, &docs).ingest(
            &dir,
            &IngestOptions::default(),
            NOW,
        );
        assert!(r.is_err(), "dimension mismatch must error");
        assert!(format!("{:#}", r.unwrap_err()).contains("dim mismatch"));
    }
}
