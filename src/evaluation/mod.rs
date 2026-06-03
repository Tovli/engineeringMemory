//! Evaluation bounded context — makes retrieval quality measurable (Hit@K, MRR, …).
//! Conformist to the Retrieval context: calls M2's SearchService through a `SearchPort`
//! and conforms to its `RetrievalRun` output. Read-only except for the JSON report it writes.
//! Hexagonal: `domain` ← `ports` ← `application` ← `infra`.
//! See docs/sparc/m3-retrieval-eval/03-architecture.md and docs/adrs/0004..0005.

pub mod domain;
pub mod ports;

pub mod application;
pub mod infra;
