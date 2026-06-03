//! Conformist seam into the Retrieval context (evaluation.md "Relationship with Retrieval").
//! Evaluation depends only on this trait + Retrieval's `RetrievalRun`/`Query` output model,
//! never on Retrieval's infrastructure.

use crate::retrieval::domain::{Query, RetrievalRun};

pub trait SearchPort {
    /// Run one query through Retrieval and return its (conformed) run.
    fn search(&self, query: &Query) -> anyhow::Result<RetrievalRun>;
}
