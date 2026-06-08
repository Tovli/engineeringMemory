//! RagAnswerService — the application core. Pipeline: weak-retrieval gate → provider availability
//! → render → call LLM → validate citations → assemble grounded Answer. Generic over `LlmPort` →
//! fully unit-testable with fake LLMs (no network). The domain (not the prompt) enforces grounding:
//! no valid citation ⇒ no answer (FR-RAG-002/003, ADR-0006).

use std::collections::BTreeSet;
use std::time::Instant;

use crate::answer_generation::application::context_assembly::{assemble, render, MAX_CONTEXT_TOKENS};
use crate::answer_generation::domain::answer::{Answer, Citation, NoAnswerReason};
use crate::answer_generation::domain::prompt_template::{self, PromptTemplate};
use crate::answer_generation::ports::{FinishReason, LlmPort};
use crate::retrieval::domain::RetrievalRun;

pub struct RagAnswerService<'a> {
    pub llm: &'a dyn LlmPort,
}

/// Identity + timestamp injected for determinism (mirrors M1/M2/M3 services).
pub struct AnswerContext<'a> {
    pub query_id: &'a str,
    pub answer_id: &'a str,
    pub now: &'a str,
    pub max_tokens: usize,
}

impl RagAnswerService<'_> {
    /// Generate an Answer from a completed RetrievalRun. Never panics; every path returns an Answer
    /// (a no-answer is still an Answer with a reason and a user-facing message).
    pub fn generate(&self, run: &RetrievalRun, ctx: &AnswerContext) -> Answer {
        let t0 = Instant::now();
        let template = prompt_template::active();

        // 1. Weak-retrieval gate (AC-2, E1/E2) — refuse BEFORE any LLM call.
        let assembled = assemble(run, MAX_CONTEXT_TOKENS);
        if assembled.chunks.is_empty() {
            return self.no_answer(
                run,
                ctx,
                &template,
                NoAnswerReason::BelowSimilarityThreshold,
                "No retrieved source cleared the similarity threshold, so I don't have a reliable basis to answer.",
                "none",
                t0,
            );
        }

        // 2. Provider availability (E6) — never call `complete` when unavailable.
        if !self.llm.is_available() {
            return self.no_answer(
                run,
                ctx,
                &template,
                NoAnswerReason::LlmProviderError,
                "The answer provider is currently unavailable. Retrieval succeeded — try again or use --no-llm.",
                "unknown",
                t0,
            );
        }

        // 3. Render + call the LLM.
        let request = render(&template, &assembled, ctx.max_tokens);
        let response = match self.llm.complete(&request) {
            Ok(r) if r.finish_reason != FinishReason::Error => r,
            Ok(r) => {
                return self.no_answer(run, ctx, &template, NoAnswerReason::LlmProviderError,
                    "The answer provider returned an error.", &r.provider, t0);
            }
            Err(_) => {
                return self.no_answer(run, ctx, &template, NoAnswerReason::LlmProviderError,
                    "The answer provider failed to produce a response.", "unknown", t0);
            }
        };
        let provider = response.provider.clone();

        // 4. Empty answer text (E5) → don't present an empty/uncited answer.
        if response.text.trim().is_empty() {
            return self.no_answer(run, ctx, &template, NoAnswerReason::OutsideCorpus,
                "The retrieved sources don't appear to answer this question.", &provider, t0);
        }

        // 5. Validate citations against the run — strip invented ids (AC-6, E4).
        let valid: BTreeSet<&str> = run.results.iter().map(|r| r.chunk_id.as_str()).collect();
        let mut cited: Vec<String> = Vec::new();
        for id in &response.cited_chunk_ids {
            if valid.contains(id.as_str()) && !cited.iter().any(|c| c == id) {
                cited.push(id.clone());
            }
        }

        // 6. No valid citation remains (E3, AC-6) → refuse rather than emit an ungrounded answer.
        if cited.is_empty() {
            return self.no_answer(run, ctx, &template, NoAnswerReason::OutsideCorpus,
                "I couldn't ground an answer in the retrieved sources.", &provider, t0);
        }

        // 7. Build citations (rank order from the run) + retrieved-but-unused list (FR-RAG-002).
        let cited_set: BTreeSet<&str> = cited.iter().map(String::as_str).collect();
        let citations: Vec<Citation> = run
            .results
            .iter()
            .filter(|r| cited_set.contains(r.chunk_id.as_str()))
            .map(|r| Citation {
                rank: r.rank,
                chunk_id: r.chunk_id.clone(),
                source_path: r.source_path.clone(),
                heading_path: r.heading_path.clone(),
                preview: r.preview.clone(),
            })
            .collect();
        let unused: Vec<String> = run
            .results
            .iter()
            .filter(|r| !cited_set.contains(r.chunk_id.as_str()))
            .map(|r| r.chunk_id.clone())
            .collect();

        // 8. Grounded answer (AC-1, AC-7 invariant: non-empty citations when no reason).
        Answer {
            id: ctx.answer_id.to_string(),
            query_id: ctx.query_id.to_string(),
            query_text: run.query.text.clone(),
            retrieval_run_id: run.id.clone(),
            prompt_template_version: template.version.clone(),
            answer_text: response.text.trim().to_string(),
            citations,
            retrieved_but_unused_chunks: unused,
            no_answer_reason: None,
            llm_provider: provider,
            latency_ms: t0.elapsed().as_millis(),
            created_at: ctx.now.to_string(),
        }
    }

    /// Build a no-answer Answer. Always stamps the prompt version (AC-4) and lists every retrieved
    /// chunk as unused (none were cited). `message` is the user-facing explanation (invariant 2).
    #[allow(clippy::too_many_arguments)]
    fn no_answer(
        &self,
        run: &RetrievalRun,
        ctx: &AnswerContext,
        template: &PromptTemplate,
        reason: NoAnswerReason,
        message: &str,
        provider: &str,
        t0: Instant,
    ) -> Answer {
        Answer {
            id: ctx.answer_id.to_string(),
            query_id: ctx.query_id.to_string(),
            query_text: run.query.text.clone(),
            retrieval_run_id: run.id.clone(),
            prompt_template_version: template.version.clone(),
            answer_text: message.to_string(),
            citations: Vec::new(),
            retrieved_but_unused_chunks: run.results.iter().map(|r| r.chunk_id.clone()).collect(),
            no_answer_reason: Some(reason),
            llm_provider: provider.to_string(),
            latency_ms: t0.elapsed().as_millis(),
            created_at: ctx.now.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use crate::answer_generation::ports::{LlmRequest, LlmResponse};
    use crate::ingestion::domain::EmbeddingModelVersion;
    use crate::retrieval::domain::{MetadataFilter, Query, RetrievalResult, RetrievalRun, RunReason, SearchMode};

    const NOW: &str = "2026-06-06T00:00:00Z";

    fn model() -> EmbeddingModelVersion {
        EmbeddingModelVersion { name: "mock".into(), dimension: 8, created_at: NOW.into() }
    }

    fn res(rank: usize, chunk: &str, score: f32) -> RetrievalResult {
        RetrievalResult {
            rank,
            chunk_id: chunk.into(),
            document_id: "d".into(),
            source_path: format!("docs/{chunk}.md"),
            score,
            preview: format!("preview of {chunk}"),
            heading_path: vec!["H".into()],
            metadata: BTreeMap::new(),
        }
    }

    fn run_with(results: Vec<RetrievalResult>) -> RetrievalRun {
        RetrievalRun {
            id: "rrun_1".into(),
            query: Query {
                text: "what are the layering rules".into(),
                mode: SearchMode::Vector,
                filters: MetadataFilter::default(),
                top_k: 5,
                embedding_model: model(),
            },
            results,
            search_mode: SearchMode::Vector,
            top_k: 5,
            latency_ms: 7,
            below_threshold_count: 0,
            reason: RunReason::Ok,
            explain: None,
            completed_at: NOW.into(),
        }
    }

    fn actx<'a>() -> AnswerContext<'a> {
        AnswerContext { query_id: "qry_1", answer_id: "ans_1", now: NOW, max_tokens: 256 }
    }

    /// Fake LLM with a configurable reply; records whether it was called (to prove the gate).
    struct FakeLlm {
        text: String,
        cited: Vec<String>,
        finish: FinishReason,
        available: bool,
        called: Cell<bool>,
    }
    impl FakeLlm {
        fn ok(text: &str, cited: &[&str]) -> Self {
            Self {
                text: text.into(),
                cited: cited.iter().map(|s| s.to_string()).collect(),
                finish: FinishReason::Stop,
                available: true,
                called: Cell::new(false),
            }
        }
    }
    impl LlmPort for FakeLlm {
        fn complete(&self, _r: &LlmRequest) -> anyhow::Result<LlmResponse> {
            self.called.set(true);
            Ok(LlmResponse {
                text: self.text.clone(),
                cited_chunk_ids: self.cited.clone(),
                finish_reason: self.finish,
                provider: "fake".into(),
                latency_ms: 0,
            })
        }
        fn is_available(&self) -> bool {
            self.available
        }
    }

    #[test]
    fn grounded_answer_has_citations_and_holds_invariant() {
        // AC-1, AC-7
        let llm = FakeLlm::ok("Layering separates concerns.", &["c1"]);
        let svc = RagAnswerService { llm: &llm };
        let run = run_with(vec![res(1, "c1", 0.90), res(2, "c2", 0.80)]);
        let ans = svc.generate(&run, &actx());

        assert!(ans.no_answer_reason.is_none());
        assert_eq!(ans.citations.len(), 1);
        assert_eq!(ans.citations[0].chunk_id, "c1");
        assert_eq!(ans.retrieved_but_unused_chunks, vec!["c2".to_string()]);
        assert!(ans.invariant_holds());
        assert_eq!(ans.prompt_template_version, prompt_template::ACTIVE_VERSION);
        assert_eq!(ans.retrieval_run_id, "rrun_1");
    }

    #[test]
    fn weak_retrieval_refuses_without_calling_the_llm() {
        // AC-2, E2 — all results below SIMILARITY_THRESHOLD (0.30).
        let llm = FakeLlm::ok("should not be used", &["c1"]);
        let svc = RagAnswerService { llm: &llm };
        let run = run_with(vec![res(1, "c1", 0.10), res(2, "c2", 0.05)]);
        let ans = svc.generate(&run, &actx());

        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::BelowSimilarityThreshold));
        assert!(ans.citations.is_empty());
        assert!(!llm.called.get(), "LLM must not be called on weak retrieval");
        assert_eq!(ans.prompt_template_version, prompt_template::ACTIVE_VERSION); // AC-4 even for no-answer
    }

    #[test]
    fn empty_run_is_below_threshold_no_answer() {
        // E1
        let llm = FakeLlm::ok("x", &["c1"]);
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![]), &actx());
        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::BelowSimilarityThreshold));
        assert!(!llm.called.get());
    }

    #[test]
    fn unavailable_provider_yields_provider_error() {
        // E6
        let mut llm = FakeLlm::ok("x", &["c1"]);
        llm.available = false;
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![res(1, "c1", 0.9)]), &actx());
        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::LlmProviderError));
        assert!(!llm.called.get(), "must not call complete when unavailable");
    }

    #[test]
    fn finish_reason_error_yields_provider_error() {
        // E6
        let mut llm = FakeLlm::ok("partial", &["c1"]);
        llm.finish = FinishReason::Error;
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![res(1, "c1", 0.9)]), &actx());
        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::LlmProviderError));
    }

    #[test]
    fn invented_citations_are_stripped_then_outside_corpus() {
        // AC-6, E4 — the only cited id is not in the run.
        let llm = FakeLlm::ok("Confident but ungrounded.", &["does-not-exist"]);
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![res(1, "c1", 0.9)]), &actx());
        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::OutsideCorpus));
        assert!(ans.citations.is_empty());
    }

    #[test]
    fn partially_invalid_citations_keep_only_the_valid_ones() {
        // AC-6, E4 — one real id + one invented id → keep the real one.
        let llm = FakeLlm::ok("Grounded answer.", &["c1", "ghost"]);
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![res(1, "c1", 0.9), res(2, "c2", 0.8)]), &actx());
        assert!(ans.no_answer_reason.is_none());
        let ids: Vec<&str> = ans.citations.iter().map(|c| c.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c1"]);
    }

    #[test]
    fn empty_answer_text_is_outside_corpus() {
        // E5 — fluent-but-empty / whitespace answer is not presented.
        let llm = FakeLlm::ok("   ", &["c1"]);
        let svc = RagAnswerService { llm: &llm };
        let ans = svc.generate(&run_with(vec![res(1, "c1", 0.9)]), &actx());
        assert_eq!(ans.no_answer_reason, Some(NoAnswerReason::OutsideCorpus));
    }
}
