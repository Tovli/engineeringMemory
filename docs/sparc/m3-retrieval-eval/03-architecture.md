# SPARC Phase 3 — Architecture: M3 Retrieval Evaluation

Rust translation of the **Evaluation bounded context** (`docs/ddd/contexts/evaluation.md`), Conformist
to Retrieval: it calls M2's `SearchService` through an internal `SearchPort` and conforms to its output
(`RetrievalRun`). Hexagonal, read-only (writes only the report file): domain ← ports ← application ←
infra; the CLI calls application only (PRD §9.4). Key decisions: **[ADR-0004](../../adrs/0004-relevance-judgment.md)**, **[ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md)**.

## Module layout
```
src/
  main.rs                       # + `Eval(EvalArgs)` subcommand — load/build/delegate/write (C5/R9)
  lib.rs                        # + `pub mod evaluation;`
  retrieval/ ...                # unchanged — reused via SearchPort (C6)
  evaluation/
    mod.rs
    domain/                     # pure types, no engine/infra deps
      question.rs               #   EvalQuestion (+ camelCase serde)
      metrics.rs                #   EvalMetrics
      question_result.rs        #   EvalQuestionResult
      run.rs                    #   EvalRun (aggregate root), EvalRunStatus, EvalRunConfig, ThresholdConfig
    ports.rs                    # SearchPort (conformist seam into Retrieval)
    application/
      relevance.rs              #   is_relevant / source_matches — pure (ADR-0004)
      metrics_calc.rs           #   compute_metrics / threshold_status — pure (FR-EVAL-002)
      evaluation_service.rs     #   EvaluationService<S: SearchPort>: per-q search → judge → metrics
    infra/
      retrieval_search_adapter.rs  # RetrievalSearchAdapter (SearchPort) — wraps retrieval::SearchService
      dataset_loader.rs            # load + validate questions.json (FR-EVAL-001, E1/E2)
      report_writer.rs             # serde_json EvalReport → --output (FR-EVAL-002, AC-5)
eval/
  questions.json              # ≥20 curated questions over the repo's docs (deliverable)
tests/
  evaluation_eval.rs          # integration: ingest → eval with MockEmbedder + crafted dataset (C7)
```

## Dependency direction (acyclic; Evaluation → Retrieval, one way)
```
                 ┌─────────────┐
 CLI (main.rs) ─▶│ application │─▶ ports(SearchPort) ─▶ domain
                 └─────────────┘                          ▲
 infra: RetrievalSearchAdapter ─implements SearchPort──────┘
                 │ wraps
                 ▼
       retrieval::application::SearchService  (M2, unchanged)
```
- `evaluation::domain` imports only shared-kernel types (`ChunkId`) — no engine crates.
- `evaluation::ports::SearchPort` returns Retrieval's `RetrievalRun` (Conformist — evaluation.md).
- `evaluation::application` depends on ports + domain; the service is generic over `SearchPort`.
- only `evaluation::infra` knows `retrieval::SearchService`, `serde_json` files. → **no cycles.**

## Typed contracts
```rust
// --- domain ---
pub struct EvalQuestion {                              // serde(rename_all = "camelCase")
    pub id: String, pub question: String,
    pub expected_chunk_ids: Vec<ChunkId>,              // default empty
    pub expected_source_files: Vec<String>,            // default empty
}
pub struct EvalMetrics {
    pub hit_at_1: f64, pub hit_at_3: f64, pub hit_at_5: f64, pub mrr: f64,
    pub avg_latency_ms: f64, pub empty_result_count: usize,
    pub below_threshold_count: usize, pub question_count: usize,
}
pub struct EvalQuestionResult {
    pub question_id: String, pub question_text: String, pub retrieval_run_id: String,
    pub returned_chunk_ids: Vec<String>, pub returned_source_paths: Vec<String>,
    pub hit_at_1: bool, pub hit_at_3: bool, pub hit_at_5: bool,
    pub reciprocal_rank: f64, pub latency_ms: u128, pub top_score: Option<f32>, pub empty: bool,
}
pub enum EvalRunStatus { Completed, ThresholdFailed, Failed }
pub struct ThresholdConfig { pub min_hit_at_3: Option<f64> }
pub struct EvalRunConfig {
    pub mode: SearchMode, pub top_k: usize,
    pub threshold: Option<ThresholdConfig>, pub embedding_model: EmbeddingModelVersion,
}
pub struct EvalRun {
    pub id: String, pub dataset_path: String, pub search_mode: SearchMode, pub top_k: usize,
    pub embedding_model: EmbeddingModelVersion, pub status: EvalRunStatus,
    pub metrics: EvalMetrics, pub question_results: Vec<EvalQuestionResult>,
    pub error: Option<String>, pub started_at: String, pub completed_at: String,
}

// --- port (conformist seam) ---
pub trait SearchPort {
    fn search(&self, query: &Query) -> anyhow::Result<RetrievalRun>;   // returns Retrieval's type
}
```
`EvaluationService<'a, S: SearchPort>` (or `&'a dyn SearchPort`) → unit-tested with a fake `SearchPort`
returning canned `RetrievalRun`s; metric math verified with **no** embedder, store, or disk.

## How decisions land
| Decision | Resolution |
|---|---|
| **D-RELEVANCE** ([ADR-0004](../../adrs/0004-relevance-judgment.md)) | `relevance.rs`: `is_relevant` = exact `chunk_id` ∈ expected, OR `source_matches` (normalize `\`→`/`, strip `./`, lowercase; equal / `ends_with("/"+exp)` / basename eq). First relevant rank only (E7). |
| **D-DEPTH/CI** ([ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md)) | service retrieves `k = max(top_k, 5)`; CI = pure metric tests + mock-embedder exact-match IT; semantic Hit@3 ≥ 0.80 verified locally with ONNX. |
| **below-threshold** | `metrics_calc` reuses `retrieval::application::scoring::SIMILARITY_THRESHOLD` — eval and search agree (ADR-0003). |
| **fatal mismatch (E6)** | a `search` error (e.g. `EmbeddingModelMismatch`) makes the service return `EvalRun{ status: Failed, error }`; CLI prints it and exits non-zero — a config error never masquerades as 0% quality. |
| **persistence** | M3 artefact is the JSON report (`report_writer`). No `eval_runs` DB table yet (evaluation.md `EvalRunRepository` deferred — the report file is sufficient for FR-EVAL-002/003). |

## Constraint coverage
- **C1/C6** reuses M2 `SearchService` + adapters unchanged; no Docker, no Retrieval-domain change.
- **C2** eval uses the same local embedder builder as `search`; no LLM (R2), no network.
- **C3** read-only except the report file; Evaluation holds only a `SearchPort`, never write ports.
- **C4** serial: `Q` searches dominate; 50 × ~tens-ms ONNX ≪ 2 min.
- **C5/R9** `main.rs` `eval` handler: load dataset, build ports, call service, write report, set exit code.
- **C7** integration test in `./tests/evaluation_eval.rs`; ADRs in `./docs/adrs/`.

## Domain events (logged, §9.3 — no bus)
`EvaluationCompleted` on finish (run id, mode, metrics, status). Consumed conceptually by reporting; for
M3 it is a structured log line.

## Phase-3 Gate (to advance → Refinement)
- [x] Architecture addresses all constraints (C1–C7).
- [x] API contracts typed (domain structs + `SearchPort` trait + serde report).
- [x] No circular dependencies (Evaluation → Retrieval one-way via SearchPort; engine crates only in infra).
- [x] Every AC has a home: AC-1 (dataset_loader + service loop), AC-2/3/4 (metrics_calc), AC-5 (report_writer), AC-6 (threshold_status + CLI exit), AC-7 (relevance.rs + loader validation), AC-8 (eval/questions.json + documented ONNX run).
- [x] The two decisions each have an owning ADR (0004/0005).

**Gate result: PASS.** → Phase 4 (Refinement: implement + tests in `./tests`).
