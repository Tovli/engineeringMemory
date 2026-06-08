//! Pure domain types for the Answer Generation context (no engine/infra deps).
//! Mirrors docs/ddd/contexts/answer-generation.md. Reuses the shared-kernel `ChunkId`.

pub mod answer;
pub mod context;
pub mod prompt_template;

pub use answer::{Answer, Citation, NoAnswerReason};
pub use context::{ContextChunk, RetrievedContext};
pub use prompt_template::PromptTemplate;
