//! Dataset loader — reads + validates the evaluation questions JSON (FR-EVAL-001, E1/E2).

use crate::evaluation::domain::question::EvalQuestion;

/// Load and validate the dataset at `path`. Errors on unreadable/malformed/empty files,
/// or any question lacking ground truth (evaluation.md EvalQuestion invariant 1).
pub fn load_dataset(path: &str) -> anyhow::Result<Vec<EvalQuestion>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read dataset '{path}': {e}"))?;
    let questions: Vec<EvalQuestion> = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("malformed dataset '{path}': {e}"))?;
    if questions.is_empty() {
        anyhow::bail!("dataset '{path}' is empty");
    }
    for q in &questions {
        if !q.has_ground_truth() {
            anyhow::bail!(
                "question '{}' has no expectedChunkIds or expectedSourceFiles (no ground truth)",
                q.id
            );
        }
    }
    Ok(questions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);
    fn tmp(content: &str) -> String {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("tovli-ds-{}-{}.json", std::process::id(), n));
        std::fs::write(&p, content).unwrap();
        p.to_string_lossy().to_string()
    }

    #[test]
    fn loads_valid_dataset() {
        let p = tmp(r#"[{"id":"q1","question":"what?","expectedSourceFiles":["docs/a.md"]}]"#);
        let qs = load_dataset(&p).unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].expected_source_files, vec!["docs/a.md".to_string()]);
        assert!(qs[0].expected_chunk_ids.is_empty());
    }

    #[test]
    fn rejects_question_without_ground_truth() {
        let p = tmp(r#"[{"id":"q1","question":"what?"}]"#);
        assert!(load_dataset(&p).unwrap_err().to_string().contains("no ground truth"));
    }

    #[test]
    fn rejects_empty_and_malformed() {
        assert!(load_dataset(&tmp("[]")).unwrap_err().to_string().contains("empty"));
        assert!(load_dataset(&tmp("not json")).unwrap_err().to_string().contains("malformed"));
    }
}
