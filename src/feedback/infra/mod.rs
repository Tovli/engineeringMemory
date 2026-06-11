//! Feedback infrastructure adapters.

pub mod jsonl_retrieval_run_log;
pub mod redb_feedback_repository;

pub use jsonl_retrieval_run_log::JsonlRetrievalRunLog;
pub use redb_feedback_repository::{export_feedback_json, RedbFeedbackRepository};
