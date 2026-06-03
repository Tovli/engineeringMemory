//! Retrieval bounded context — natural-language question → ranked Chunks.
//! Read-only consumer of the index the Ingestion context populates.
//! Hexagonal: `domain` (pure types) ← `ports` (traits) ← `application` ← `infra`.
//! See docs/sparc/m2-retrieval-cli/03-architecture.md and docs/adrs/0001..0003.

pub mod domain;
pub mod ports;

pub mod application;
pub mod infra;
