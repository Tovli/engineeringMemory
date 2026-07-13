//! Feedback ports. Application services depend on these, not concrete storage.

use crate::feedback::domain::{FeedbackItem, FeedbackQuery, RetrievalRunEvidence};

pub trait FeedbackRepository {
    fn save(&self, item: &FeedbackItem) -> anyhow::Result<()>;
    fn save_many(&self, items: &[FeedbackItem]) -> anyhow::Result<()>;
    fn find_by_id(&self, id: &str) -> anyhow::Result<Option<FeedbackItem>>;
    fn find_all(&self, query: Option<FeedbackQuery>) -> anyhow::Result<Vec<FeedbackItem>>;
}

pub trait RetrievalRunEvidenceStore {
    fn append(&self, evidence: &RetrievalRunEvidence) -> anyhow::Result<()>;
    fn find_by_run_id(
        &self,
        retrieval_run_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>>;
    fn find_latest_by_query_id(
        &self,
        query_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>>;
}
