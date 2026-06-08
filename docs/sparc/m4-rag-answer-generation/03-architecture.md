# SPARC Phase 3 — Architecture: M4 RAG Answer Generation

Rust translation of the **Answer Generation bounded context** (`docs/ddd/contexts/answer-generation.md`).
It consumes a completed `RetrievalRun` (read-only) and turns it into a cited `Answer`; it never queries
the vector store (PRD §7 "Retrieval Before Generation"). Hexagonal: domain ← ports ← application ← infra;
the CLI calls application only (PRD §9.4). Key decisions: **[ADR-0006](../../adrs/0006-llm-provider-abstraction.md)** (LlmPort seam), **[ADR-0007](../../adrs/0007-prompt-template-versioning.md)** (prompt versioning), **[ADR-0008](../../adrs/0008-citation-protocol-and-no-answer-policy.md)** (citation protocol + no-answer/exit policy).

## Module layout
```
src/
  main.rs                       # + `Ask(AskArgs)` subcommand — retrieve, delegate, print, log (C5/R12)
  lib.rs                        # + `pub mod answer_generation;`
  retrieval/ ...                # unchanged — reused for the one search call (C6)
  answer_generation/
    mod.rs
    domain/                     # pure types, no engine/infra deps
      answer.rs                 #   Answer (aggregate root) + Citation + NoAnswerReason (+ camelCase serde)
      context.rs                #   RetrievedContext + ContextChunk
      prompt_template.rs        #   PromptTemplate + active() (versioned, in source — ADR-0007)
    ports.rs                    # LlmPort (ACL) + LlmRequest/LlmResponse/FinishReason
    application/
      context_assembly.rs       #   assemble() (threshold filter + budget trim) + render() — pure
      rag_service.rs            #   RagAnswerService<L: LlmPort>: gate → call → validate → Answer
    infra/
      mock_llm.rs               # MockLlm (deterministic default provider, ADR-0006)
      answer_log_writer.rs      # append_answer_log() — JSONL, camelCase (R9/AC-4)
tests/
  answer_generation_ask.rs    # integration: ingest → retrieve → generate with MockEmbedder+MockLlm
```

## Dependency direction (acyclic; Answer Generation → Retrieval, one way)
```
                 ┌─────────────┐
 CLI (main.rs) ─▶│ application │─▶ ports(LlmPort) ─▶ domain
   │ retrieves   └─────────────┘        ▲
   │ (M2 SearchService, unchanged)      │ implements
   ▼                                    │
 RetrievalRun ──fed into──▶ RagAnswerService     infra: MockLlm ─┘
```
- `answer_generation::domain` imports only the shared-kernel `ChunkId` and (in application) Retrieval's
  read-only `RetrievalRun` — no engine crates.
- `application` depends on `ports::LlmPort` + domain; the service is generic over the port.
- only `infra` knows a concrete provider (`MockLlm`) and the filesystem (`serde_json` JSONL). → **no cycles.**
- The CLI is the only place that wires retrieval **and** generation together; the context itself holds
  just an `&dyn LlmPort` — no search ports, no write ports (read-only, C3).

## Typed contracts
```rust
// --- domain ---
pub enum NoAnswerReason { BelowSimilarityThreshold, OutsideCorpus, SourcesConflict, LlmProviderError } // camelCase serde
pub struct Citation { pub rank: usize, pub chunk_id: ChunkId, pub source_path: String,
                      pub heading_path: Vec<String>, pub preview: String }
pub struct Answer {
    pub id: String, pub query_id: String, pub query_text: String, pub retrieval_run_id: String,
    pub prompt_template_version: String, pub answer_text: String, pub citations: Vec<Citation>,
    pub retrieved_but_unused_chunks: Vec<ChunkId>, pub no_answer_reason: Option<NoAnswerReason>,
    pub llm_provider: String, pub latency_ms: u128, pub created_at: String,
}
impl Answer { fn invariant_holds(&self) -> bool { self.no_answer_reason.is_some() || !self.citations.is_empty() } }
pub struct ContextChunk { pub rank: usize, pub chunk_id: ChunkId, pub source_path: String,
                          pub heading_path: Vec<String>, pub text: String, pub score: f32 }
pub struct RetrievedContext { pub query_text: String, pub chunks: Vec<ContextChunk> }
pub struct PromptTemplate { pub version: String, pub system_prompt: String,
                            pub context_template: String, pub instructions: String }

// --- port (ACL over the LLM provider) ---
pub enum FinishReason { Stop, Length, Error }
pub struct LlmRequest  { pub system_prompt: String, pub user_message: String, pub max_tokens: usize }
pub struct LlmResponse { pub text: String, pub cited_chunk_ids: Vec<String>,
                         pub finish_reason: FinishReason, pub provider: String, pub latency_ms: u128 }
pub trait LlmPort { fn complete(&self, r: &LlmRequest) -> anyhow::Result<LlmResponse>; fn is_available(&self) -> bool; }
```
`RagAnswerService<'a> { llm: &'a dyn LlmPort }` with `generate(&self, run: &RetrievalRun, ctx: &AnswerContext)
-> Answer` → unit-tested with fake `LlmPort`s; the citation/no-answer logic is verified with **no**
embedder, store, network, or disk.

## How decisions land
| Decision | Resolution |
|---|---|
| **LLM seam** ([ADR-0006](../../adrs/0006-llm-provider-abstraction.md)) | `ports::LlmPort`; domain holds no SDK/keys; `MockLlm` is the default; a real provider is a future feature-gated `infra` adapter behind the same trait. |
| **D-PROMPT-VERSION** ([ADR-0007](../../adrs/0007-prompt-template-versioning.md)) | `prompt_template::active()` is a const-in-source template with an immutable `ACTIVE_VERSION` ("v1.0.0"); the version is stamped into **every** Answer (incl. no-answer), so the log is regression-queryable. |
| **D-CITATION-PROTOCOL** ([ADR-0008](../../adrs/0008-citation-protocol-and-no-answer-policy.md)) | renderer tags each chunk `[[chunk:<id>]]` and instructs a trailing `SOURCES:` line; the adapter parses it into `LlmResponse.cited_chunk_ids`; the service validates those against the run (strip invented ids → `outsideCorpus` if none survive). |
| **citation invariant** | `RagAnswerService` builds citations only from validated run chunk ids; `citations` empty ⇔ `no_answer_reason` set — enforced in code, not prompt (PRD §7, FR-RAG-002). |
| **below-threshold gate** | `context_assembly` reuses `retrieval::application::scoring::SIMILARITY_THRESHOLD` — search, eval, RAG agree (ADR-0003). |
| **D-CONFLICT** ([ADR-0008](../../adrs/0008-citation-protocol-and-no-answer-policy.md)) | `SourcesConflict` exists in the type but is **deferred** in M4 (reliable contradiction detection needs a richer LLM contract); cases fall through to a normal cited answer, never a silent merge. |
| **D-EXITCODE** ([ADR-0008](../../adrs/0008-citation-protocol-and-no-answer-policy.md)) | a no-answer is a valid product response → exit 0; only `LlmProviderError` exits non-zero (3), so scripts/CI distinguish "no reliable source" from "provider broken". |
| **persistence** | M4 artefact is the JSONL answer log (`answer_log_writer`); the DDD `AnswerRepository` query methods are deferred (the log file is sufficient for FR-RAG-004). |

## Constraint coverage
- **C1/C6** reuses M2 `SearchService` + adapters unchanged; no Docker, no Retrieval-domain change.
- **C2** the same local embedder builder as `search`/`eval`; the default provider (`MockLlm`) is offline.
- **C3** read-only except the answer log; the context holds only an `&dyn LlmPort`, never write/search ports.
- **C4** `SIMILARITY_THRESHOLD` imported from `retrieval::application::scoring` (single source of truth).
- **C5/R12** `main.rs` `ask` handler: retrieve, build provider, call `RagAnswerService`, print, log, set exit.
- **C7** `--mode vector` only (M5 guard, same as M2/M3).
- **C8** integration test in `./tests/answer_generation_ask.rs`; ADRs in `./docs/adrs/`; SPARC in `./docs/sparc/`.
- **C9** the LLM call is the only heavy step; `--no-llm` removes it (retrieval-only ≈ `search` speed).

## Domain events (logged, §9.3 — no bus)
`AnswerGenerated` on finish (answer id, run id, prompt version, citation count or no-answer reason). For M4
it is a structured line in the JSONL answer log (one record per `ask`).

## Phase-3 Gate (to advance → Refinement)
- [x] Architecture addresses all constraints (C1–C9).
- [x] API contracts typed (domain structs + `LlmPort` trait + `LlmRequest`/`LlmResponse` + serde Answer).
- [x] No circular dependencies (Answer Generation → Retrieval one-way; engine crates / provider only in infra).
- [x] Every AC has a home: AC-1 (rag_service + print_answer), AC-2 (assemble gate), AC-3 (print_context),
      AC-4 (prompt_template + answer_log_writer), AC-5 (CLI `--no-llm`), AC-6/AC-7 (rag_service validation),
      AC-8 (MockLlm + integration test).
- [x] The open decisions each have an owning ADR (0006 seam, 0007 prompt version, 0008 citation/no-answer/exit).

**Gate result: PASS.** → Phase 4 (Refinement: implement + tests in `./tests`).
