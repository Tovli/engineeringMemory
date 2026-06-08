# ADR-0006: LLM provider behind an `LlmPort` seam, domain enforces citations

- **Status:** Accepted
- **Date:** 2026-06-06
- **Milestone:** M4 — RAG Answer Generation
- **Context refs:** `docs/ddd/contexts/answer-generation.md` (`LlmPort` ACL, `RagAnswerService`), PRD §8.6 FR-RAG-001..004, §7 ("Retrieval Before Generation", "Sources Are Mandatory"), §15 Risk 3 (hallucination), ADR-0001 (hexagonal seam pattern), `src/ingestion/ports.rs` (`Embedder` precedent)

## Context
M4 adds `tovli ask`: generate an answer from a completed RetrievalRun. The answer must cite its
sources, refuse when retrieval is weak, and never hallucinate (PRD §7, FR-RAG-002/003). The LLM
backend is the one piece that is **external, non-deterministic, and untrusted for correctness** —
it may be a local model, a cloud API, or absent in CI. This is the same shape M1/M2 already solved
for embeddings: the `Embedder` trait (`src/ingestion/ports.rs`) is a provider seam with a
deterministic `MockEmbedder` so `cargo test` stays offline. RAG needs the equivalent for generation.

Two questions the abstraction must answer:
1. **Where does the LLM live in the architecture** — is it a domain dependency, or kept at arm's length?
2. **Who enforces the citation/no-answer invariants** — the prompt (i.e. we trust the LLM), or the code?

Options considered:
1. Call an SDK (OpenAI/Ollama) directly from the application service.
2. Define a narrow **`LlmPort` trait** in the domain; concrete adapters live in `infra/` behind
   features; a `MockLlm` adapter drives tests. The domain validates citations independently of the LLM.

## Decision
Add **`src/answer_generation/`** as a fourth hexagonal module mirroring `ingestion/`, `retrieval/`,
`evaluation/` (snake_case, `domain/ ← ports.rs ← application/ ← infra/`):

- **`LlmPort` (read-only ACL):** `complete(LlmRequest) -> LlmResponse` and `is_available() -> bool`.
  `LlmRequest { system_prompt, user_message, max_tokens }`; `LlmResponse { text, cited_chunk_ids,
  finish_reason, provider, latency_ms }`. The domain **never** holds SDK client objects or API keys —
  those live entirely in the adapter (PRD §15 Risk 3).
- **Adapters in `infra/`, feature-gated** like the ONNX embedder: a real provider (local or API)
  behind a cargo feature, plus a deterministic **`MockLlm`** that is the default for `cargo test`
  (echoes a templated answer with caller-supplied citation ids — no network, no flakiness).
- **The domain owns the invariants, not the prompt.** `RagAnswerService` (generic over `LlmPort`):
  short-circuits to `noAnswerReason = belowSimilarityThreshold` **before** calling the LLM when no
  RetrievalResult clears `retrieval::application::scoring::SIMILARITY_THRESHOLD` (single source of
  truth — same constant as search/eval, per ADR-0003/0005); strips any cited chunk id not present in
  the RetrievalRun; converts an answer with zero surviving citations to `noAnswerReason = outsideCorpus`;
  if `is_available()` is false, returns `noAnswerReason = llmProviderError` without calling `complete`.
- **Reuse the shared kernel** (`ChunkId`, `QueryId`, `SearchMode`, `EmbeddingModelVersion`) and
  **consume Retrieval's output** (RetrievalRun / `SearchExecuted`) read-only — Answer Generation never
  queries the vector store itself ("Retrieval Before Generation", PRD §7).
- `PromptTemplate` versioning (FR-RAG-004) is a separate decision — see [ADR-0007](0007-prompt-template-versioning.md). The citation protocol + no-answer/exit policy are in [ADR-0008](0008-citation-protocol-and-no-answer-policy.md). This ADR only fixes the provider seam.

## Consequences
- **+** The LLM is swappable and mockable: `cargo test` stays offline and deterministic, matching the
  `Embedder`/`MockEmbedder` precedent; real providers are an opt-in feature, not a test dependency.
- **+** Hallucination risk (PRD §15 Risk 3) is contained by **code, not prompt wording** — the
  citation-non-empty and no-invented-citations invariants hold regardless of what the LLM emits.
- **+** "Sources Are Mandatory" and the no-answer policy are enforced by the type system + service
  logic, independent of provider, and the threshold agrees with search/eval (no drift).
- **−** The LLM is forced to emit *structured* cited chunk ids (`LlmResponse.cited_chunk_ids`), which
  constrains prompt design and needs robust parsing; a chatty model that ignores the format yields
  zero valid citations → `outsideCorpus`. Acceptable: failing closed is the desired behavior.
- **−** A fourth module that opens the same redb/RuVector data read-side; same trade-off already
  accepted in ADR-0001. Adds `pub mod answer_generation;` to `src/lib.rs`.
- **Future (M5):** `SearchMode` is forwarded from the Query, so hybrid retrieval feeds RAG with no
  change to `LlmPort` or the answer invariants.
