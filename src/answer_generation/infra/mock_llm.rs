//! MockLlm — the deterministic, offline default provider (ADR-0006). It implements the citation
//! protocol (ADR-0008) from the LLM side: it reads the `[[chunk:<id>]]` tags the renderer emitted
//! and cites the first `max_citations` of them. No network, no randomness → `cargo test` and the
//! default `tovli ask` are fully reproducible. A real (cloud/local) provider is a future
//! feature-gated adapter behind this same `LlmPort` seam.

use crate::answer_generation::ports::{FinishReason, LlmPort, LlmRequest, LlmResponse};

pub struct MockLlm {
    /// How many of the rendered chunks to cite (the mock "uses" the top-ranked ones).
    pub max_citations: usize,
}

impl Default for MockLlm {
    fn default() -> Self {
        Self { max_citations: 3 }
    }
}

/// Extract chunk ids from the rendered prompt's `[[chunk:<id>]]` tags, in order, de-duplicated.
fn parse_chunk_ids(message: &str) -> Vec<String> {
    const OPEN: &str = "[[chunk:";
    let mut ids = Vec::new();
    let mut rest = message;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        match after.find("]]") {
            Some(end) => {
                let id = after[..end].to_string();
                if !id.is_empty() && !ids.contains(&id) {
                    ids.push(id);
                }
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    ids
}

impl LlmPort for MockLlm {
    fn complete(&self, request: &LlmRequest) -> anyhow::Result<LlmResponse> {
        let all = parse_chunk_ids(&request.user_message);
        let cited: Vec<String> = all.into_iter().take(self.max_citations).collect();
        // Deterministic, non-fabricated text: it only states how many sources grounded the reply.
        let text = if cited.is_empty() {
            String::new()
        } else {
            format!(
                "Based on the retrieved documentation, here is an answer grounded in {} source(s). \
                 See the cited sources for details.",
                cited.len()
            )
        };
        Ok(LlmResponse {
            text,
            cited_chunk_ids: cited,
            finish_reason: FinishReason::Stop,
            provider: "mock-llm".to_string(),
            latency_ms: 0,
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(user: &str) -> LlmRequest {
        LlmRequest { system_prompt: "sys".into(), user_message: user.into(), max_tokens: 128 }
    }

    #[test]
    fn cites_chunk_ids_parsed_from_the_prompt() {
        let llm = MockLlm::default();
        let r = llm.complete(&req("Sources:\n[[chunk:c1]] (source=a)\nx\n\n[[chunk:c2]] (source=b)\ny")).unwrap();
        assert_eq!(r.cited_chunk_ids, vec!["c1".to_string(), "c2".to_string()]);
        assert!(!r.text.is_empty());
        assert_eq!(r.finish_reason, FinishReason::Stop);
        assert_eq!(r.provider, "mock-llm");
    }

    #[test]
    fn respects_max_citations_cap() {
        let llm = MockLlm { max_citations: 1 };
        let r = llm.complete(&req("[[chunk:a]] [[chunk:b]] [[chunk:c]]")).unwrap();
        assert_eq!(r.cited_chunk_ids, vec!["a".to_string()]);
    }

    #[test]
    fn no_tags_yields_empty_answer() {
        let llm = MockLlm::default();
        let r = llm.complete(&req("no chunk tags here")).unwrap();
        assert!(r.cited_chunk_ids.is_empty());
        assert!(r.text.is_empty());
    }

    #[test]
    fn is_always_available() {
        assert!(MockLlm::default().is_available());
    }
}
