//! Query value object + search mode + metadata filter (PRD §11.4, FR-SRCH-002).

use crate::ingestion::domain::EmbeddingModelVersion;
use serde::{Deserialize, Serialize};

/// Search strategy (FR-SRCH-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Vector,
    Keyword,
    Hybrid,
}

impl std::fmt::Display for SearchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchMode::Vector => write!(f, "vector"),
            SearchMode::Keyword => write!(f, "keyword"),
            SearchMode::Hybrid => write!(f, "hybrid"),
        }
    }
}

impl std::str::FromStr for SearchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "vector" => Ok(SearchMode::Vector),
            "keyword" => Ok(SearchMode::Keyword),
            "hybrid" => Ok(SearchMode::Hybrid),
            other => Err(format!(
                "invalid search mode '{other}'; expected vector, keyword, or hybrid"
            )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn search_mode_parses_all_m5_modes() {
        assert_eq!(SearchMode::from_str("vector").unwrap(), SearchMode::Vector);
        assert_eq!(
            SearchMode::from_str("keyword").unwrap(),
            SearchMode::Keyword
        );
        assert_eq!(SearchMode::from_str("hybrid").unwrap(), SearchMode::Hybrid);
        assert_eq!(SearchMode::Keyword.to_string(), "keyword");
        assert_eq!(SearchMode::Hybrid.to_string(), "hybrid");
    }

    #[test]
    fn search_mode_rejects_invalid_value_with_accepted_values() {
        let err = SearchMode::from_str("semantic").unwrap_err();
        assert!(err.contains("semantic"));
        assert!(err.contains("vector, keyword, or hybrid"));
    }
}
