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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, dim: usize) -> EmbeddingModelVersion {
        EmbeddingModelVersion { name: name.into(), dimension: dim, created_at: "t".into() }
    }

    #[test]
    fn empty_query_message() {
        assert_eq!(RetrievalError::EmptyQuery.to_string(), "query must not be empty");
    }

    #[test]
    fn invalid_topk_message() {
        assert!(RetrievalError::InvalidTopK.to_string().contains("top-k"));
    }

    #[test]
    fn mismatch_message_names_both_models_dims_and_remediation() {
        // The user-facing message must surface both models, both dimensions, and the fix.
        let e = RetrievalError::EmbeddingModelMismatch {
            indexed: model("minilm", 384),
            query: model("mock-deterministic", 8),
        };
        let s = e.to_string();
        assert!(s.contains("minilm") && s.contains("384"), "names indexed model: {s}");
        assert!(s.contains("mock-deterministic") && s.contains('8'), "names query model: {s}");
        assert!(s.contains("reembed"), "suggests re-indexing: {s}");
    }
}
