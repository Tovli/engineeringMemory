//! Domain errors for retrieval (PRD Risk 5 / FR-EMB-002, spec R6/AC-7, E1/E2).

use crate::ingestion::domain::EmbeddingModelVersion;

/// Errors that abort a search before it produces a RetrievalRun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalError {
    /// The query string was empty / whitespace-only (E1).
    EmptyQuery,
    /// `top_k` was zero (E2).
    InvalidTopK,
    /// Query embedder model/dimension differs from the indexed model (AC-7, E8).
    /// Never silently mix dimensions (PRD Risk 5).
    EmbeddingModelMismatch { indexed: EmbeddingModelVersion, query: EmbeddingModelVersion },
}

impl std::fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetrievalError::EmptyQuery => write!(f, "query must not be empty"),
            RetrievalError::InvalidTopK => write!(f, "top-k must be a positive integer"),
            RetrievalError::EmbeddingModelMismatch { indexed, query } => write!(
                f,
                "embedding model mismatch: index was built with '{}' (dim {}) but the query embedder is '{}' (dim {}). \
                 Re-index with `tovli reembed --model {}` or query with the matching model.",
                indexed.name, indexed.dimension, query.name, query.dimension, indexed.name
            ),
        }
    }
}

impl std::error::Error for RetrievalError {}
