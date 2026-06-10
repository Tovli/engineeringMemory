//! ContextAssemblyService — turns a RetrievalRun into a token-budgeted RetrievedContext and
//! renders it into an LlmRequest (answer-generation.md). Pure: no external calls. Eligibility is
//! mode-aware: the vector similarity threshold applies only to vector-mode scores.

use crate::answer_generation::domain::context::{ContextChunk, RetrievedContext};
use crate::answer_generation::domain::prompt_template::PromptTemplate;
use crate::answer_generation::ports::LlmRequest;
use crate::retrieval::application::scoring::score_clears_similarity_threshold;
use crate::retrieval::domain::RetrievalRun;

// M5: the numeric similarity threshold is calibrated for vector scores only.
// Keyword and hybrid relevance scores pass through this gate unless there are no results.

/// Default context budget (whitespace-word estimate). Previews are short, so this rarely binds;
/// the trim logic (E7) exists so an over-large context can never blow the prompt.
pub const MAX_CONTEXT_TOKENS: usize = 1500;

/// Rough token estimate — same whitespace-word heuristic the ingestion chunker uses.
fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Build the context from the run's eligible results, rank-ordered, trimmed to `max_tokens` by
/// dropping the lowest-rank chunks first (E7). Keeps at least one chunk if any is eligible.
pub fn assemble(run: &RetrievalRun, max_tokens: usize) -> RetrievedContext {
    let mut chunks: Vec<ContextChunk> = run
        .results
        .iter()
        .filter(|r| score_clears_similarity_threshold(run.search_mode, r.score))
        .map(|r| ContextChunk {
            rank: r.rank,
            chunk_id: r.chunk_id.clone(),
            source_path: r.source_path.clone(),
            heading_path: r.heading_path.clone(),
            text: r.preview.clone(),
            score: r.score,
        })
        .collect();

    let mut total: usize = chunks.iter().map(|c| estimate_tokens(&c.text)).sum();
    while chunks.len() > 1 && total > max_tokens {
        if let Some(dropped) = chunks.pop() {
            total -= estimate_tokens(&dropped.text);
        }
    }

    RetrievedContext {
        query_text: run.query.text.clone(),
        chunks,
    }
}

/// Render the context into a prompt. Each chunk is tagged `[[chunk:<id>]]` so the LLM can cite by
/// id and the adapter can map the `SOURCES:` line back to chunk ids (the citation protocol, ADR-0008).
pub fn render(template: &PromptTemplate, ctx: &RetrievedContext, max_tokens: usize) -> LlmRequest {
    let rendered_chunks = ctx
        .chunks
        .iter()
        .map(|c| {
            let heading = if c.heading_path.is_empty() {
                String::new()
            } else {
                format!(" [{}]", c.heading_path.join(" > "))
            };
            format!(
                "[[chunk:{}]] (source={}){}\n{}",
                c.chunk_id, c.source_path, heading, c.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let body = template
        .context_template
        .replace("{{chunks}}", &rendered_chunks)
        .replace("{{question}}", &ctx.query_text);

    LlmRequest {
        system_prompt: template.system_prompt.clone(),
        user_message: format!("{body}\n\n{}", template.instructions),
        max_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer_generation::domain::prompt_template;
    use crate::ingestion::domain::EmbeddingModelVersion;
    use crate::retrieval::domain::{
        MetadataFilter, Query, RetrievalResult, RetrievalRun, RunReason, SearchMode,
    };
    use std::collections::BTreeMap;

    fn model() -> EmbeddingModelVersion {
        EmbeddingModelVersion {
            name: "mock".into(),
            dimension: 8,
            created_at: "t".into(),
        }
    }

    fn res(rank: usize, chunk: &str, score: f32, text: &str) -> RetrievalResult {
        RetrievalResult {
            rank,
            chunk_id: chunk.into(),
            document_id: "d".into(),
            source_path: "docs/a.md".into(),
            score,
            preview: text.into(),
            heading_path: vec!["H".into()],
            metadata: BTreeMap::new(),
        }
    }

    fn run_with(results: Vec<RetrievalResult>) -> RetrievalRun {
        run_with_mode(SearchMode::Vector, results)
    }

    fn run_with_mode(mode: SearchMode, results: Vec<RetrievalResult>) -> RetrievalRun {
        RetrievalRun {
            id: "rr".into(),
            query: Query {
                text: "what is layering".into(),
                mode,
                filters: MetadataFilter::default(),
                top_k: 5,
                embedding_model: model(),
            },
            results,
            search_mode: mode,
            top_k: 5,
            latency_ms: 1,
            below_threshold_count: 0,
            reason: RunReason::Ok,
            explain: None,
            completed_at: "t".into(),
        }
    }

    #[test]
    fn assemble_drops_below_threshold_results() {
        // E2 contributor: only score ≥ SIMILARITY_THRESHOLD survives into the context.
        let run = run_with(vec![
            res(1, "c1", 0.90, "alpha"),
            res(2, "c2", 0.10, "beta"), // below 0.30 → excluded
        ]);
        let ctx = assemble(&run, MAX_CONTEXT_TOKENS);
        let ids: Vec<&str> = ctx.chunks.iter().map(|c| c.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c1"]);
    }

    #[test]
    fn assemble_does_not_apply_vector_threshold_to_keyword_runs() {
        let run = run_with_mode(SearchMode::Keyword, vec![res(1, "c1", 0.10, "lexical hit")]);
        let ctx = assemble(&run, MAX_CONTEXT_TOKENS);
        let ids: Vec<&str> = ctx.chunks.iter().map(|c| c.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c1"]);
    }

    #[test]
    fn assemble_trims_to_token_budget_keeping_best_ranked() {
        // E7: with a tiny budget, the lowest-rank chunk is dropped first; at least one survives.
        let run = run_with(vec![
            res(1, "c1", 0.90, "one two three"),
            res(2, "c2", 0.80, "four five six"),
        ]);
        let ctx = assemble(&run, 3); // budget = 3 words → only the rank-1 chunk fits
        assert_eq!(ctx.chunks.len(), 1);
        assert_eq!(ctx.chunks[0].chunk_id, "c1");
    }

    #[test]
    fn render_tags_chunks_and_substitutes_question() {
        let run = run_with(vec![res(1, "chk_42", 0.90, "layering rules")]);
        let ctx = assemble(&run, MAX_CONTEXT_TOKENS);
        let req = render(&prompt_template::active(), &ctx, 256);
        assert!(req.user_message.contains("[[chunk:chk_42]]"));
        assert!(req.user_message.contains("what is layering"));
        assert!(req.user_message.contains("SOURCES:"));
        assert!(!req.system_prompt.is_empty());
    }
}
