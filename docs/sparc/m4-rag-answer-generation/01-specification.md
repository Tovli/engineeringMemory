# SPARC Phase 1 — Specification: M4 RAG Answer Generation

Feature: **M4 — RAG Answer Generation** · Slug: `m4-rag-answer-generation` · Started: 2026-06-06
Derived from PRD §8.6 (FR-RAG-001..004), §12.4 (`ask` CLI), §7 (principles), §13 Milestone 4, §9 (NFR), §15 Risk 3.
Bounded context: **Answer Generation** (`docs/ddd/contexts/answer-generation.md`) — consumes Retrieval's
`RetrievalRun` read-only; never queries the vector store directly ("Retrieval Before Generation", §7).
Architecture seam fixed by **[ADR-0006](../../adrs/0006-llm-provider-abstraction.md)** (`LlmPort`).
Stack continues M0–M3: Rust + `ruvector-core` + `redb` + local ONNX/MiniLM (or mock) + a feature-gated LLM.

## Goal
`tovli ask "..."` runs the question through the M2 retrieval pipeline, assembles the retrieved chunks
into a versioned prompt, calls an LLM behind `LlmPort`, and prints a **concise answer with mandatory
`Sources:` citations** — or a clear **no-answer response** when retrieval is weak, the corpus doesn't
cover the question, or the provider is unavailable. The domain (not the prompt) enforces that every
answer is grounded: no citations ⇒ no answer. `--show-context` makes retrieval visible; `--no-llm`
runs retrieval-only. This is where the M3-measured retrieval quality is finally turned into cited prose.

## Scope (Milestone 4)
**In:** `tovli ask` · `LlmPort` provider abstraction + `MockLlm` (default) + one feature-gated real
provider · versioned prompt templates in source control · context assembly from a `RetrievalRun` ·
citation formatting (`Sources:` list) · no-answer behavior (below-threshold / outside-corpus /
provider-error) · `--show-context` · `--no-llm` (retrieval-only) · persisted answer logs with prompt
version. Reuses M2 `SearchService` via a `SearchPort`.
**Out (later):** keyword/hybrid mode → **M5** (`--mode` accepts only `vector` in M4, same guard as
M2/M3; `SearchMode` is forwarded so M5 needs no RAG change); feedback on answers → **M6**; HTTP API →
**M7**; bot → **M8**. Persisting answer logs to a relational table is out — the `redb`/JSONL answer log
is the M4 artefact.

## Requirements
- **R1** (FR-RAG-001) `tovli ask "question"` retrieves chunks via the M2 pipeline, then generates an answer from **only** those retrieved chunks.
- **R2** (FR-RAG-001, §12.4) Only retrieved chunks are sent to the LLM; the system injects no external knowledge into the context.
- **R3** (FR-RAG-002) Every answer includes a `Sources:` list of the chunks it used, formatted as `N. source/path.md#chunk-id`.
- **R4** (FR-RAG-002, §7 "Sources Are Mandatory") **Domain invariant:** an Answer without a `noAnswerReason` must carry ≥ 1 Citation, and every cited `ChunkId` must exist in the originating `RetrievalRun` (no invented citations). Enforced in code, independent of the LLM.
- **R5** (FR-RAG-003) No-answer behavior with an explicit, user-facing reason (never an empty answer): `belowSimilarityThreshold`, `outsideCorpus`, `sourcesConflict` (best-effort, see D-CONFLICT), `llmProviderError`.
- **R6** (FR-RAG-002, §12.4) `--show-context` prints the retrieved chunks (rank, source path, score, preview); retrieved-but-**unused** chunks are surfaced in this debug view.
- **R7** (§12.4) `--no-llm` runs retrieval-only: prints the context and exits **without** constructing the LLM provider or generating an answer.
- **R8** (FR-RAG-004) Prompt templates live in source control with an immutable `version`; each persisted answer log records the prompt version used (even for no-answer responses).
- **R9** (FR-RAG-001, answer-generation.md `Answer` aggregate) Persist an answer log per attempt: `queryId`, `retrievalRunId`, `promptTemplateVersion`, `answerText`, `citations`, `retrievedButUnusedChunks`, `noAnswerReason?`, `llmProvider`, `latencyMs`, `createdAt`.
- **R10** ([ADR-0006](../../adrs/0006-llm-provider-abstraction.md)) The LLM sits behind `LlmPort` (`complete`, `is_available`); the domain holds no SDK clients or API keys. `MockLlm` is the default for `cargo test`; a real provider is feature-gated like the ONNX embedder.
- **R11** (§7 "Retrieval Before Generation", answer-generation.md) Answer Generation consumes a `RetrievalRun` read-only and never touches the vector store/embedder directly beyond what M2 already does.
- **R12** (§9.4) The CLI `ask` handler is thin: build ports + call `RagAnswerService` + format output. No generation, citation, or threshold logic in the handler.
- **R13** (§12.4) `--top-k` and `--mode` are forwarded to retrieval; `--mode` accepts only `vector` in M4.

## Acceptance Criteria
- **AC-1** `tovli ask "..."` over an ingested corpus prints a concise answer followed by a `Sources:` list referencing the chunks used. *(Milestone 4 AC "Answers include source references"; FR-RAG-002.)*
- **AC-2** When **all** retrieval scores are below `SIMILARITY_THRESHOLD`, the system returns a no-answer response (`belowSimilarityThreshold`) with a short explanation and does **not** call the LLM. *(Milestone 4 AC "Weak retrieval produces no-answer"; FR-RAG-003.)*
- **AC-3** `tovli ask "..." --show-context` prints the retrieved chunks (rank, source, score, preview), including retrieved-but-unused ones. *(Milestone 4 AC "context can be shown"; §12.4.)*
- **AC-4** The prompt-template `version` is stored in the persisted answer log for every attempt, including no-answer responses. *(Milestone 4 AC "Prompt version is stored"; FR-RAG-004.)*
- **AC-5** `tovli ask "..." --no-llm` runs retrieval only and prints context without generating an answer or requiring LLM provider config. *(§12.4 "Can run retrieval-only mode".)*
- **AC-6** A cited `ChunkId` not present in the `RetrievalRun` is **stripped**; if no valid citation remains, the result becomes `noAnswerReason = outsideCorpus` rather than an uncited answer. *(FR-RAG-002 invariant; answer-generation.md invariant 4; [ADR-0006](../../adrs/0006-llm-provider-abstraction.md).)*
- **AC-7** An Answer with no `noAnswerReason` always has ≥ 1 Citation — enforced by the service, not by prompt wording. *(§7 "Sources Are Mandatory"; answer-generation.md invariant 1.)*
- **AC-8** With `MockLlm`, `cargo test` produces a deterministic cited answer fully offline (no network); the real provider compiles only behind its cargo feature. *(Milestone 4 + [ADR-0006](../../adrs/0006-llm-provider-abstraction.md); CI determinism, cf. [ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md).)*

## Constraints
- **C1** Rust edition 2021; MSVC toolchain for the ONNX/real-LLM paths; pure-Rust subset for mock-only builds. **No Docker.**
- **C2** Local-first (§7): the default `ask` runs offline with `MockLlm`; external LLMs are optional and feature-gated. The domain holds **no** API keys or SDK clients ([ADR-0006](../../adrs/0006-llm-provider-abstraction.md)).
- **C3** Read-only on retrieval/index: Answer Generation reads a `RetrievalRun` and never writes the index or mutates embeddings; the only things it writes are the answer log and stdout.
- **C4** Reuse `retrieval::application::scoring::SIMILARITY_THRESHOLD` for the weak-retrieval gate — single source of truth, same constant as search/eval ([ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)/[0005](../../adrs/0005-eval-depth-and-ci-determinism.md)); no drift.
- **C5** Clippy-clean; the `ask` handler delegates to `RagAnswerService` (R12).
- **C6** Reuse the M2 `SearchService` + read adapters unchanged; no changes to the Retrieval or Ingestion domain.
- **C7** `--mode` accepts only `vector` in M4 (keyword/hybrid → M5); same guard as M2/M3.
- **C8** Tests live in `./tests`; ADRs in `./docs/adrs`; SPARC docs in `./docs/sparc`. *(Project convention.)*
- **C9** Performance (NFR §9): a single `ask` is dominated by one retrieval + one LLM call; the `--no-llm` path adds zero LLM latency. No added per-query latency budget beyond the LLM round-trip.

## Edge Cases
- **E1** Empty index / zero retrieved results → no-answer (`belowSimilarityThreshold`, counts as empty result); never a crash. *(feeds from retrieval E3.)*
- **E2** Some results returned but **all** below threshold → no-answer **before** any LLM call (no wasted round-trip). *(AC-2.)*
- **E3** LLM returns a fluent answer but **zero** citations → `outsideCorpus`; never present an uncited answer. *(answer-generation.md no-answer flow.)*
- **E4** LLM cites chunk ids that are not in the `RetrievalRun` → strip the invalid ids; keep the answer iff ≥ 1 valid citation remains, else `outsideCorpus`. *(AC-6.)*
- **E5** LLM returns valid citations but empty/whitespace `answerText` → treated as no usable answer → `outsideCorpus`.
- **E6** Provider unavailable (`is_available()` false) or `complete` errors → `llmProviderError` with a user-facing message; the domain never calls `complete` when `is_available()` is false. *(answer-generation.md `LlmPort` ACL.)*
- **E7** Assembled context exceeds the token budget → `ContextAssemblyService` trims lowest-rank chunks first; the answer log records which chunks were actually sent vs `retrievedButUnusedChunks`.
- **E8** `--no-llm` with a healthy index → prints context and exits 0; must not construct the LLM provider nor fail on missing provider config. *(AC-5, E6 must not trigger.)*
- **E9** `--mode keyword|hybrid` in M4 → rejected with the same clear "vector only" error as M2/M3. *(C7.)*
- **E10** Embedding-model mismatch at query time (M2 AC-7) → **fatal** abort with the actionable message — a config error must not masquerade as "no reliable source found". *(don't let it look like a quality miss.)*
- **E11** Retrieved chunks plausibly contradict each other → best-effort `sourcesConflict` (see **D-CONFLICT**); if conflict detection is deferred, such cases fall through to a normal cited answer, never a silent merge.

## Open Decisions (to be resolved in Architecture, recorded as ADRs)
- **D-PROMPT-VERSION** How prompt templates are stored and versioned (file location under source control; semver string vs git hash; how the active template is selected). Satisfies FR-RAG-004 / R8 / AC-4. *(Likely the second M4 ADR.)*
- **D-CITATION-PROTOCOL** The concrete wire format by which the LLM emits structured `cited_chunk_ids` and how the adapter parses them into `LlmResponse.cited_chunk_ids` (trailing machine-readable block vs structured/JSON output). [ADR-0006](../../adrs/0006-llm-provider-abstraction.md) mandates structured citation ids; the protocol is the open part.
- **D-CONFLICT** Whether `sourcesConflict` (FR-RAG-003) is detected in M4 (e.g. an LLM-emitted conflict signal mapped by the domain) or explicitly deferred to a later milestone. Affects R5/E11.
- **D-EXITCODE** Exit-code semantics: is a no-answer response (a valid product outcome) exit 0, while only `llmProviderError`/fatal config errors are non-zero? Affects scripting/CI use of `ask`.

## Phase-1 Gate (criteria for advancing to Pseudocode)
- [x] ≥ 3 acceptance criteria — **8** (AC-1…AC-8), each traced to a PRD FR / Milestone-4 AC.
- [x] Explicit constraints — **C1…C9**.
- [x] Edge cases identified — **E1…E11**, including the non-obvious ones (E2 no wasted LLM call, E4 citation stripping, E7 token-budget trim, E10 fatal model mismatch).
- [x] Requirements trace to PRD §8.6 / §12.4 / §7 / Milestone 4 / §9, and to [ADR-0006](../../adrs/0006-llm-provider-abstraction.md).

**Gate result: PASS (pending `/sparc advance`).** → Phase 2 (Pseudocode).
