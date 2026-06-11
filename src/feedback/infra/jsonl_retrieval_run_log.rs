//! JSONL retrieval-run evidence log. This keeps the displayed-result snapshot needed by
//! later `tovli feedback` commands without coupling Feedback to Retrieval internals.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::feedback::domain::RetrievalRunEvidence;
use crate::feedback::ports::RetrievalRunEvidenceStore;

pub struct JsonlRetrievalRunLog {
    path: PathBuf,
}

impl JsonlRetrievalRunLog {
    pub fn open(path: &str) -> Self {
        Self {
            path: PathBuf::from(path),
        }
    }

    pub fn append(&self, evidence: &RetrievalRunEvidence) -> anyhow::Result<()> {
        <Self as RetrievalRunEvidenceStore>::append(self, evidence)
    }

    pub fn find_by_run_id(
        &self,
        retrieval_run_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>> {
        <Self as RetrievalRunEvidenceStore>::find_by_run_id(self, retrieval_run_id)
    }

    pub fn find_latest_by_query_id(
        &self,
        query_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>> {
        <Self as RetrievalRunEvidenceStore>::find_latest_by_query_id(self, query_id)
    }

    fn read_all(&self) -> anyhow::Result<Vec<RetrievalRunEvidence>> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let body = std::fs::read_to_string(&self.path)?;
        let mut out = Vec::new();
        for (i, line) in body.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let evidence = serde_json::from_str(line)
                .map_err(|e| anyhow::anyhow!("invalid retrieval-run log line {}: {e}", i + 1))?;
            out.push(evidence);
        }
        Ok(out)
    }
}

impl RetrievalRunEvidenceStore for JsonlRetrievalRunLog {
    fn append(&self, evidence: &RetrievalRunEvidence) -> anyhow::Result<()> {
        if let Some(parent) = Path::new(&self.path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let line = serde_json::to_string(evidence)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    fn find_by_run_id(
        &self,
        retrieval_run_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>> {
        Ok(self
            .read_all()?
            .into_iter()
            .rev()
            .find(|run| run.retrieval_run_id == retrieval_run_id))
    }

    fn find_latest_by_query_id(
        &self,
        query_id: &str,
    ) -> anyhow::Result<Option<RetrievalRunEvidence>> {
        Ok(self
            .read_all()?
            .into_iter()
            .rev()
            .find(|run| run.query_id == query_id))
    }
}
