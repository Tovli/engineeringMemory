//! Infrastructure adapters implementing the ingestion ports.

pub mod parsers;
pub mod mock_embedder;
pub mod redb_repo;
pub mod ruvector_store;

#[cfg(feature = "onnx")]
pub mod onnx_embedder;
