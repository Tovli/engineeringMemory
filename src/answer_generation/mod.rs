//! Answer Generation (RAG) bounded context — turns a completed `RetrievalRun` into a cited,
//! grounded answer (PRD §8.6 FR-RAG-001..004). Consumes Retrieval's output read-only; it never
//! queries the vector store itself ("Retrieval Before Generation", PRD §7). The LLM sits behind
//! an `LlmPort` ACL so the domain holds no SDK/keys and tests run offline with `MockLlm`.
//! Hexagonal: `domain` ← `ports` ← `application` ← `infra`.
//! See docs/sparc/m4-rag-answer-generation/ and docs/adrs/0006..0008.

pub mod domain;
pub mod ports;

pub mod application;
pub mod infra;
