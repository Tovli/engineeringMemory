//! Read-only ports (seams) for the Retrieval context — traits depending only on `domain`
//! and shared-kernel types. Infra implements these; application depends on them, never on infra.
//! See ADR-0001 (bounded context), ADR-0002 (document-lookup join).

use std::collections::HashMap;

use crate::ingestion::domain::{DocumentId, DocumentStatus, EmbeddingModelVersion};

/// ACL-internal shape returned by the vector store — never exposed to the domain.
/// `distance` is the raw cosine distance (lower = closer); the application
/// normalizes it to a [0,1] similarity (ADR-0003).
#[derive(Debug, Clone)]
pub struct RawSearchResult {
    pub chunk_id: String,
    pub document_id: String,
    pub source_path: String,
    pub distance: f32,
    pub preview: String,
    pub heading_path: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

/// Document metadata needed to apply project/tag filters and drop deleted docs (ADR-0002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocMeta {
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub source_path: String,
    pub status: DocumentStatus,
}

/// Read-only vector search over the index Ingestion populated (VectorStorePort, read side).
pub trait VectorSearchPort {
    /// Top-`k` nearest neighbours by cosine distance. No native metadata filter is applied
    /// (filtering is done in the application layer — ADR-0002); over-fetch happens in the caller.
    fn vector_search(&self, query_vec: &[f32], k: usize) -> anyhow::Result<Vec<RawSearchResult>>;
}

/// Read-only document metadata lookup (over `documents.redb`).
pub trait DocumentLookupPort {
    /// Resolve metadata for the given document ids (ADR-0002 join).
    fn find_many(&self, ids: &[DocumentId]) -> anyhow::Result<HashMap<DocumentId, DocMeta>>;

    /// The embedding model the active index was built with, or `None` when no active
    /// document exists (empty index → AC-8). Used for the model-compatibility guard (R6/AC-7).
    fn indexed_model_version(&self) -> anyhow::Result<Option<EmbeddingModelVersion>>;
}
