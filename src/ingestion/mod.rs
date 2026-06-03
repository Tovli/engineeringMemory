//! Ingestion bounded context — raw file on disk → indexed vector in RuVector.
//! Hexagonal: `domain` (pure types) ← `ports` (traits) ← application ← `infra`.
//! See docs/sparc/m1-document-ingestion/03-architecture.md.

pub mod domain;
pub mod ports;

pub mod chunking;
pub mod embedding;
pub mod model_guard;
pub mod orchestrator;

pub mod infra;
