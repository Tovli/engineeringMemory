//! tovli library — RuVector-based technical memory assistant.
//!
//! The CLI bin (`src/main.rs`) is a thin shell over this library (PRD §9.4).

pub mod vector_store;
pub mod ingestion;
pub mod retrieval;
pub mod evaluation;
pub mod answer_generation;
