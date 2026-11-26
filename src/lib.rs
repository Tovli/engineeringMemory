//! tovli library — RuVector-based technical memory assistant.
//!
//! The CLI bin (`src/main.rs`) is a thin shell over this library (PRD §9.4).

pub mod answer_generation;
pub mod evaluation;
pub mod feedback;
pub mod image_memory;
pub mod ingestion;
pub mod lexical_index;
pub mod retrieval;
pub mod vector_store;
pub mod store_lock;
