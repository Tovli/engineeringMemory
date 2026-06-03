//! Query value object + search mode + metadata filter (PRD §11.4, FR-SRCH-002).

use crate::ingestion::domain::EmbeddingModelVersion;

/// Search strategy. M2 implements `Vector`; `Keyword`/`Hybrid` arrive in M5
/// (the seam exists so they plug in without touching the domain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Vector,
}

impl std::fmt::Display for SearchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchMode::Vector => write!(f, "vector"),
        }
    }
}

/// Query-time metadata filters (FR-SRCH-002). All set filters AND together.
/// `tags` is multi-valued: a document must carry every requested tag (ADR-0002).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataFilter {
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub source: Option<String>,
}

impl MetadataFilter {
    pub fn is_empty(&self) -> bool {
        self.project.is_none() && self.tags.is_empty() && self.source.is_none()
    }
}

/// One search request. Immutable once created (PRD §11.4 invariant 3).
#[derive(Debug, Clone)]
pub struct Query {
    pub text: String,
    pub mode: SearchMode,
    pub filters: MetadataFilter,
    pub top_k: usize,
    /// The model the query will be embedded with; checked against the index (R6).
    pub embedding_model: EmbeddingModelVersion,
}
