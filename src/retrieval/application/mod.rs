//! Application layer for the Retrieval context — orchestrates the read ports.

pub mod filters;
pub mod scoring;
pub mod search_service;

pub use search_service::SearchService;
