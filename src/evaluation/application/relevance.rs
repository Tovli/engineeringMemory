//! Relevance judgment — pure, no I/O (ADR-0004, AC-7, edge case E4).

use crate::evaluation::domain::question::EvalQuestion;
use crate::retrieval::domain::RetrievalResult;

/// Is this result relevant to the question? Exact chunk-id match OR path-tolerant source match.
pub fn is_relevant(q: &EvalQuestion, result: &RetrievalResult) -> bool {
    if q.expected_chunk_ids.iter().any(|c| c == &result.chunk_id) {
        return true;
    }
    q.expected_source_files.iter().any(|exp| source_matches(&result.source_path, exp))
}

/// Tolerant path comparison: normalize separators, strip leading `./`, lowercase; then match on
/// equality, suffix (`indexed` ends with `/expected`), or basename equality (ADR-0004).
pub fn source_matches(indexed: &str, expected: &str) -> bool {
    let a = normalize(indexed);
    let b = normalize(expected);
    if a == b {
        return true;
    }
    if a.ends_with(&format!("/{b}")) {
        return true;
    }
    basename(&a) == basename(&b)
}

fn normalize(p: &str) -> String {
    let s = p.replace('\\', "/");
    let s = s.strip_prefix("./").unwrap_or(&s);
    s.to_lowercase()
}

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn result(chunk: &str, source: &str) -> RetrievalResult {
        RetrievalResult {
            rank: 1,
            chunk_id: chunk.into(),
            document_id: "d".into(),
            source_path: source.into(),
            score: 0.9,
            preview: "p".into(),
            heading_path: vec![],
            metadata: BTreeMap::new(),
        }
    }
    fn q(chunks: &[&str], sources: &[&str]) -> EvalQuestion {
        EvalQuestion {
            id: "q1".into(),
            question: "?".into(),
            expected_chunk_ids: chunks.iter().map(|s| s.to_string()).collect(),
            expected_source_files: sources.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn exact_chunk_id_matches() {
        assert!(is_relevant(&q(&["chunk_abc"], &[]), &result("chunk_abc", "x.md")));
        assert!(!is_relevant(&q(&["chunk_abc"], &[]), &result("chunk_zzz", "x.md")));
    }

    #[test]
    fn source_match_is_path_tolerant() {
        // windows-separated indexed path vs posix dataset path, with leading ./
        assert!(source_matches("./docs\\arch.md", "docs/arch.md"));
        // suffix match
        assert!(source_matches("/abs/repo/docs/ddd/contexts/retrieval.md", "contexts/retrieval.md"));
        // basename fallback
        assert!(source_matches("a/b/auth.md", "z/auth.md"));
        // case-insensitive
        assert!(source_matches("docs/README.md", "readme.md"));
        // genuine mismatch
        assert!(!source_matches("docs/auth.md", "docs/deploy.md"));
    }

    #[test]
    fn relevant_when_any_expected_source_matches() {
        let question = q(&[], &["docs/arch.md", "docs/deploy.md"]);
        assert!(is_relevant(&question, &result("c1", "./docs/deploy.md")));
        assert!(!is_relevant(&question, &result("c1", "./docs/other.md")));
    }
}
