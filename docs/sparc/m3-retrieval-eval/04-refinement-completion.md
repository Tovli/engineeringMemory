# SPARC Phases 4–5 — Refinement & Completion: M3 Retrieval Evaluation

Implementation of the Phase-3 architecture (TDD, London-school for the service core).
Status: **complete** — `tovli eval` works end-to-end; 51 tests green (incl. all integration suites
under parallel execution); clippy clean.

## What was built
```
src/evaluation/
  mod.rs
  domain/{mod,question,metrics,question_result,run}.rs            # pure types (+ camelCase serde)
  ports.rs                                                        # SearchPort (conformist seam)
  application/{mod,relevance,metrics_calc,evaluation_service}.rs  # core + pure units
  infra/{mod,retrieval_search_adapter,dataset_loader,report_writer}.rs
src/main.rs        # + `Eval(EvalArgs)` subcommand, run_eval, print_eval (thin — C5/R9)
src/lib.rs         # + `pub mod evaluation;`
eval/questions.json         # 22 curated questions over the repo's own docs (deliverable, AC-1/AC-8)
tests/evaluation_eval.rs    # integration: ingest → eval with MockEmbedder (deterministic, ADR-0005)
docs/adrs/{0004,0005}.md
```

## Test results
- `cargo test` (parallel, default) → **51 passed** (lib 46 incl. ingestion/retrieval/evaluation unit
  tests; `evaluation_eval` 2; `retrieval_search` 3), 0 failed.
- `cargo clippy --all-targets` → **no issues**.
- CLI smoke (`tovli eval`, mock embedder): prints Hit@1/3/5 + MRR + latency + counts; writes camelCase
  JSON report; `--fail-below-hit-at-3` returns exit 0 when satisfied and exit 1 when not.

## ⚠️ Refinement-phase finding: HNSW pre-allocation OOM (fixed)
Running the integration suites **in parallel** aborted with `memory allocation of ~3 GB failed`
(`STATUS_STACK_BUFFER_OVERRUN`). Root cause: `ruvector_core::types::HnswConfig::default()` sets
`max_elements = 10_000_000`, which pre-allocates several GB **per `VectorDB` instance**. A single
instance fits (the M2 CLI smoke worked), but concurrent test instances exhausted memory.

- **This latently affected the M2 `retrieval_search` integration test too** — it only surfaced now
  because rtk's compacted summary reported the lib suite's "passed" count and the crashed integration
  binary's abort was easy to miss. Lesson: verify per-binary `test result:` lines, not just a rolled-up
  total.
- **Fix:** a shared `vector_store::default_hnsw_config()` with `MAX_INDEX_ELEMENTS = 100_000`
  (20× the PRD §9.1 5,000-chunk target, a fraction of the memory), used by all three `VectorDB`
  open sites (M0 `RuVectorStore`, ingestion `RuVectorStoreAdapter`, retrieval `RuVectorSearchAdapter`).
- Existing local `.tovli` stores built before this change should be rebuilt (`tovli ingest`) since the
  index capacity config changed; stores are local/gitignored so no migration is needed.

## Traceability matrix (acceptance criterion → test)
| AC | Covered by |
|----|-----------|
| AC-1 ≥20 questions load & run | `eval/questions.json` (22 Qs); IT `end_to_end_eval_computes_metrics_and_writes_report`; UT `dataset_loader::loads_valid_dataset` |
| AC-2 Hit@3 computed | UT `metrics_calc::computes_hits_mrr_latency`; IT (asserts `hitAt3`) |
| AC-3 MRR computed | UT `metrics_calc::computes_hits_mrr_latency` (MRR = 0.5 case); IT (`mrr > 0`) |
| AC-4 Hit@1/5, latency, empty, below-threshold | UT `metrics_calc::*`; `evaluation_service::empty_results_count_as_miss_and_empty` |
| AC-5 JSON report written | IT (writes report, re-reads `questionCount`/`searchMode`/`questionResults`); CLI smoke |
| AC-6 `--fail-below-hit-at-3` exit code | UT `metrics_calc::threshold_gate_strict_below_fails_equal_passes`; `evaluation_service::threshold_failure_sets_status_and_no_error`; IT `threshold_failure_when_expected_file_absent`; CLI smoke (exit 1) |
| AC-7 relevance judgment + load validation | UT `relevance::*` (exact id, path-tolerant, basename); `dataset_loader::rejects_question_without_ground_truth` |
| AC-8 Hit@3 ≥ 0.80 (semantic) | `eval/questions.json` + documented local ONNX run (`tovli ingest ./docs && tovli eval ./eval/questions.json`); **not** CI-gated (ADR-0005) |
| E6 fatal model mismatch | UT `evaluation_service::fatal_search_error_aborts_with_failed_status` |

(UT = unit test inline in `src/`; IT = integration test in `tests/`.)

## Deviations / decisions during implementation
- **EvalRunRepository deferred.** evaluation.md describes persisting EvalRuns; M3's artefact is the
  JSON report file (sufficient for FR-EVAL-002/003). No `eval_runs` DB table yet.
- **`MIN_EVAL_K = 5`** enforced in `EvaluationService` so Hit@5/MRR are always computable regardless
  of `--top-k` (ADR-0005).
- **Mode comparison (FR-EVAL-002)** is structurally supported (EvalRun records the mode; reports are
  diffable) but only `vector` exists until M5 — same guard as M2.

## Phase-4 Gate (Refinement)
- [x] Every acceptance criterion has a passing test (matrix above).
- [x] Code review: Evaluation depends on Retrieval one-way via `SearchPort`; no engine crates in
      `evaluation::domain`; `main.rs` `eval` handler delegates to `EvaluationService` (C5/R9).
- [x] Real-stack integration test passes **under parallel execution** after the HNSW OOM fix.

## Phase-5 Gate (Completion)
- [x] All tests green (51) + clippy clean (parallel run verified).
- [x] Read-only invariant (C3): Evaluation only reads Retrieval output; the sole write is the report file.
- [x] Docs complete: spec / pseudocode / architecture / 2 ADRs / this completion record.
- [x] Traceability matrix complete (above).
- [ ] **Deferred / manual:** the semantic Hit@3 ≥ 0.80 target (AC-8) is verified by a local ONNX run,
      not in CI (ADR-0005). Run `tovli ingest ./docs && tovli eval ./eval/questions.json` with the
      `onnx` feature to measure it.

**SPARC workflow for M3 — Retrieval Evaluation: COMPLETE.**
