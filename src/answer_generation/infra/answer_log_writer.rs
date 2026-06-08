//! Answer-log writer (FR-RAG-004 / R9) — appends each Answer as one JSON line (JSONL) so logs
//! accumulate across `tovli ask` runs and stay queryable by prompt version for regression review.
//! The prompt version is part of the serialized Answer (camelCase), matching the eval report style.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::answer_generation::domain::answer::Answer;

/// Append `answer` as a single JSON line to `path`, creating parent directories as needed.
pub fn append_answer_log(path: &str, answer: &Answer) -> anyhow::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let line = serde_json::to_string(answer)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer_generation::domain::answer::{Citation, NoAnswerReason};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmp_path() -> String {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir()
            .join(format!("tovli-ans-{}-{}.jsonl", std::process::id(), n))
            .to_string_lossy()
            .to_string()
    }

    fn answer(reason: Option<NoAnswerReason>, citations: Vec<Citation>) -> Answer {
        Answer {
            id: "ans_1".into(),
            query_id: "qry_1".into(),
            query_text: "q".into(),
            retrieval_run_id: "rrun_1".into(),
            prompt_template_version: "v1.0.0".into(),
            answer_text: "a".into(),
            citations,
            retrieved_but_unused_chunks: vec![],
            no_answer_reason: reason,
            llm_provider: "mock-llm".into(),
            latency_ms: 3,
            created_at: "2026-06-06T00:00:00Z".into(),
        }
    }

    fn citation(chunk: &str) -> Citation {
        Citation {
            rank: 1,
            chunk_id: chunk.into(),
            source_path: "docs/a.md".into(),
            heading_path: vec!["H".into()],
            preview: "p".into(),
        }
    }

    #[test]
    fn appends_camel_case_json_lines_with_prompt_version() {
        let path = tmp_path();
        append_answer_log(&path, &answer(None, vec![citation("c1")])).unwrap();
        append_answer_log(&path, &answer(Some(NoAnswerReason::OutsideCorpus), vec![])).unwrap();

        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["promptTemplateVersion"], "v1.0.0"); // AC-4: camelCase, version present
        assert_eq!(first["citations"][0]["chunkId"], "c1");
        assert!(first.get("noAnswerReason").is_none(), "omitted when None");

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["noAnswerReason"], "outsideCorpus");

        let _ = std::fs::remove_file(&path);
    }
}
