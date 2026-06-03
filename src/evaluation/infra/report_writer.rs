//! Report writer — serializes an EvalRun to a structured JSON report (FR-EVAL-002, AC-5).

use std::path::Path;

use serde::Serialize;

use crate::ingestion::domain::EmbeddingModelVersion;
use crate::evaluation::domain::metrics::EvalMetrics;
use crate::evaluation::domain::question_result::EvalQuestionResult;
use crate::evaluation::domain::run::{EvalRun, EvalRunStatus};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalReport<'a> {
    run_id: &'a str,
    generated_at: &'a str,
    dataset_path: &'a str,
    search_mode: String,
    top_k: usize,
    status: EvalRunStatus,
    embedding_model: &'a EmbeddingModelVersion,
    metrics: &'a EvalMetrics,
    question_results: &'a [EvalQuestionResult],
}

/// Write the JSON report for `run` to `path`, creating parent directories as needed.
pub fn write_report(path: &str, run: &EvalRun) -> anyhow::Result<()> {
    let report = EvalReport {
        run_id: &run.id,
        generated_at: &run.completed_at,
        dataset_path: &run.dataset_path,
        search_mode: run.search_mode.to_string(),
        top_k: run.top_k,
        status: run.status,
        embedding_model: &run.embedding_model,
        metrics: &run.metrics,
        question_results: &run.question_results,
    };
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(path, json)?;
    Ok(())
}
