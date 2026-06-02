//! Ports (seams) for the Ingestion context — traits depending only on `domain`.
//! Infrastructure implements these; application depends on them (never on infra).

use crate::ingestion::domain::{Chunk, DocumentId, EmbeddingModelVersion, IngestionDocument};

/// Output of parsing a raw file into text.
#[derive(Debug, Clone)]
pub struct ParsedDoc {
    pub text: String,
    pub title: Option<String>,
}

/// Converts raw file bytes into text for a set of extensions (FileParserPort).
pub trait FileParser {
    /// Extensions (without dot) this parser handles, e.g. `["md"]`.
    fn extensions(&self) -> &'static [&'static str];
    /// Parse raw bytes; Err on non-UTF8/binary or malformed input (edge case E3).
    fn parse(&self, raw: &[u8]) -> anyhow::Result<ParsedDoc>;
}

/// Generates embedding vectors for text (EmbeddingPort ACL).
pub trait Embedder {
    fn model_version(&self) -> &EmbeddingModelVersion;
    /// Each returned vector has length == `model_version().dimension`.
    fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// A chunk paired with its embedding, ready to index.
pub struct ChunkWithEmbedding<'a> {
    pub chunk: &'a Chunk,
    pub vector: Vec<f32>,
}

/// Persists chunk vectors in the vector store (VectorStorePort ACL — RuVector).
pub trait VectorStorePort {
    fn upsert_chunks(&self, items: &[ChunkWithEmbedding]) -> anyhow::Result<()>;
    fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()>;
}

/// Persists document records + enables idempotency/incremental detection (redb).
pub trait DocumentRepository {
    fn find_by_path(&self, path: &str) -> anyhow::Result<Option<IngestionDocument>>;
    fn save(&self, doc: &IngestionDocument) -> anyhow::Result<()>;
    fn soft_delete(&self, id: &DocumentId) -> anyhow::Result<()>;
    /// All active documents whose source_path is under `root` (for deletion detection).
    fn active_under(&self, root: &str) -> anyhow::Result<Vec<IngestionDocument>>;
}
