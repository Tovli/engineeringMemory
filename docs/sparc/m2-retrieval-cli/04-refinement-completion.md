# SPARC Phases 4–5 — Refinement & Completion: M2 Retrieval CLI

Implementation of the Phase-3 architecture (TDD, London-school for the service core).
Status: **complete** — `tovli search` works end-to-end; 32 tests green; clippy clean.

## What was built
```
src/retrieval/
  mod.rs
  domain/{mod,query,retrieval_result,retrieval_run,explain,errors}.rs   # pure types
  ports.rs                                                              # VectorSearchPort, DocumentLookupPort, RawSearchResult, DocMeta
  application/{mod,scoring,filters,search_service}.rs                   # core pipeline + pure units
  infra/{mod,ruvector_search,redb_lookup}.rs                           # read-only adapters
src/main.rs        # + `Search(SearchArgs)` subcommand, run_search, formatter (thin — C5/R9)
src/lib.rs         # + `pub mod retrieval;`
tests/retrieval_search.rs   # integration: ingest → search through real ruvector-core + redb
docs/adrs/{README,0001,0002,0003}.md
```

## Test results
- `cargo test` → **32 passed** (lib unit suite + 3 integration tests), 0 failed.
- `cargo clippy --all-targets` → **no issues**.
- CLI smoke (mock embedder, 3-doc corpus): ranked output with source/score/heading/preview/chunk-id;
  `--explain` payload; project filter; "no results for these filters"; empty-index message;
  `--mode hybrid` rejected with exit 2.

> The mock embedder is non-semantic (blake3-derived), so smoke-test *ranking* isn't meaningful —
> it validates plumbing, formatting, filtering, and guards. Semantic ranking quality is M3's job
> (evaluation) with the ONNX embedder.

## Traceability matrix (acceptance criterion → test)
| AC | Covered by |
|----|-----------|
| AC-1 ranked results | IT `search_returns_ranked_results_with_source_and_score`; UT `returns_ranked_results_best_first`; CLI smoke |
| AC-2 source + score, non-increasing | same as AC-1 (asserts `windows` non-increasing, score ∈ [0,1], rank 1 first) |
| AC-3 top-k trim | UT `trims_to_top_k` |
| AC-4 project/tag/source filters | UT `filters_by_project_tag_source`; IT `project_and_tag_and_source_filters_apply` |
| AC-5 no results (graceful) | UT `no_results_after_filter_is_ok_run`; IT (nonexistent project); CLI smoke |
| AC-6 explain mode | UT `explain_payload_is_populated_when_requested`; CLI smoke |
| AC-7 model mismatch errors, writes nothing | UT `model_mismatch_is_an_error_and_runs_nothing` |
| AC-8 empty index | UT `empty_index_yields_index_empty_reason_not_error`; IT `empty_index_reports_index_empty`; CLI smoke |
| E1/E2 empty query / top-k 0 | UT `empty_query_and_zero_topk_are_rejected` |
| E5 over-fetch on filter | UT `scoring::fetch_k_overfetches_only_when_filtering` |
| E7 distance→similarity, clamped | UT `scoring::similarity_is_inverted_and_clamped` |
| E9 deleted doc excluded | UT `deleted_document_is_excluded` |

(UT = unit test inline in `src/`; IT = integration test in `tests/`.)

## Deviations from the Phase-3 design (reconciled back into the docs)
- **`indexed_model_version()` moved from `VectorSearchPort` → `DocumentLookupPort`.** The
  authoritative model/dimension lives in `documents.redb`, and reading it there lets the CLI size
  the vector store by the indexed dimension *before* opening it (no dimension-mismatch at open).
  Architecture port block + Model-guard row + pseudocode step 1 updated to match.
- **`fetch_k` dropped its index-size cap.** The store returns ≤ what it holds, so capping at `N`
  was unnecessary; over-fetch stays bounded at `top_k × OVERFETCH`. Reflected in `scoring.rs`.
- **ADR-0003 assumption verified:** `scoring::similarity_is_inverted_and_clamped` pins
  `similarity = 1 − distance` (distance 0 → 1.0, distance 1 → 0.0); the >1 / <0 clamp guards
  non-unit-norm vectors.

## Phase-4 Gate (Refinement)
- [x] Every acceptance criterion has a passing test (matrix above).
- [x] Code review: hexagonal boundaries hold — `domain` imports no engine crates; `ruvector-core`/`redb`
      appear only in `retrieval/infra`; `main.rs` `search` handler delegates to `SearchService` (C5/R9).
- [x] Coverage of the three surfaced risks (over-fetch, document-join filter, score normalization).

## Phase-5 Gate (Completion)
- [x] All tests green (32) + clippy clean.
- [x] Read-only invariant (C3): Retrieval implements only read ports; no write path reachable.
- [x] Docs complete: spec / pseudocode / architecture / 3 ADRs / this completion record.
- [x] Traceability matrix complete (above).
- [ ] **Deferred to M3:** semantic-quality validation (Hit@K / MRR) with the ONNX embedder — that is
      Milestone 3's objective, not M2's. M2's bar is "ranked results + filters + explain", which is met.

**SPARC workflow for M2 — Retrieval CLI: COMPLETE.**
