//! RetrievalSearchAdapter — implements the Evaluation `SearchPort` by delegating to M2's
//! retrieval `SearchService`. This is the conformist boundary: it is the only place Evaluation
//! touches Retrieval's application layer.

use crate::evaluation::ports::SearchPort;
use crate::retrieval::application::SearchService;
use crate::retrieval::domain::{Query, RetrievalRun};

pub struct RetrievalSearchAdapter<'a> {
    pub inner: SearchService<'a>,
}

impl SearchPort for RetrievalSearchAdapter<'_> {
    fn search(&self, query: &Query) -> anyhow::Result<RetrievalRun> {
        // explain is off for evaluation; ids/timestamps are generated per call.
        let run_id = format!("rrun_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let now = chrono::Utc::now().to_rfc3339();
        self.inner.search(query, false, &run_id, &now)
    }
}
