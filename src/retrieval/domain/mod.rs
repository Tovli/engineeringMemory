//! Pure domain types for the Retrieval context (no engine/infra deps).
//! Mirrors docs/ddd/contexts/retrieval.md. Reuses shared-kernel value objects
//! (`DocumentId`, `ChunkId`, `DocumentStatus`, `EmbeddingModelVersion`) from `ingestion::domain`.

pub mod errors;
pub mod explain;
pub mod query;
pub mod retrieval_result;
pub mod retrieval_run;

pub use errors::RetrievalError;
pub use explain::{ExplainPayload, ExplainResultDetail};
pub use query::{MetadataFilter, Query, SearchMode};
pub use retrieval_result::RetrievalResult;
pub use retrieval_run::{RetrievalRun, RunReason};
