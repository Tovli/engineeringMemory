# SPARC Phase 1 — Specification: M3 Retrieval Evaluation

Feature: **M3 — Retrieval Evaluation** · Slug: `m3-retrieval-eval` · Started: 2026-06-03
Derived from PRD §8.7 (FR-EVAL-*), §11.4 (Query), §12.5 (`eval` CLI), §13 Milestone 3, §9.1 (perf).
Bounded context: **Evaluation** (`docs/ddd/contexts/evaluation.md`) — Conformist to Retrieval; reads
the output of M2's `SearchService`, never the vector store directly.
Stack continues M0–M2: Rust + `ruvector-core` + `redb` + local ONNX/MiniLM (or mock).

## Goal
`tovli eval ./eval/questions.json` runs each ground-truth question through the M2 retrieval pipeline,
compares the ranked results against expected chunks/source files, computes Hit@1/3/5 + MRR + latency,
prints a summary, writes a JSON report, and (optionally) exits non-zero when Hit@3 is below a
threshold — so retrieval quality is **measured, not trusted** before the RAG layer (M4) is built.

## Scope (Milestone 3)
**In:** JSON dataset format · Hit@1/3/5 · MRR · avg latency · empty-result & below-threshold counts ·
JSON report · CI failure threshold (`--fail-below-hit-at-3`). Reuses M2 `SearchService` via a `SearchPort`.
**Out (later):** keyword/hybrid mode comparison → **M5** (eval is built mode-aware so M5 only adds modes);
answer-quality eval → **M4**; feedback-driven analysis → **M6**; persisting EvalRuns to a DB table
(the JSON report is the M3 artefact). `--mode` accepts only `vector` in M3 (same guard as M2).

## Requirements
- **R1** (FR-EVAL-001) Load a JSON dataset: array of `{id, question, expectedChunkIds?, expectedSourceFiles?}`; at least one of the two `expected*` fields is required per question.
- **R2** (FR-EVAL-001) Evaluation runs **without** any LLM/answer generation (retrieval-only).
- **R3** (FR-EVAL-002) Compute Hit@1, Hit@3, Hit@5 as fractions in [0,1] over the dataset.
- **R4** (FR-EVAL-002) Compute MRR (mean over questions of 1/rank-of-first-relevant, 0 if none).
- **R5** (FR-EVAL-002) Compute average retrieval latency (ms), empty-result count, below-threshold count.
- **R6** (FR-EVAL-002) Print metrics and save a structured JSON report to `--output` (default `./eval/report.json`).
- **R7** (FR-EVAL-003) `--fail-below-hit-at-3 <x>`: exit non-zero when `hitAt3 < x`; exit 0 otherwise (CI regression gate).
- **R8** (Conformist, evaluation.md) Evaluation calls Retrieval's `SearchService` through an internal `SearchPort`; it does not import Retrieval infra types nor touch the vector store/embedder directly beyond constructing the query.
- **R9** (PRD §9.4) The CLI `eval` handler is thin: load dataset + build ports + call `EvaluationService` + write report. No metric logic in the handler.
- **R10** (PRD §7, §9.3) The report exposes per-question results (returned chunk ids/sources, hits, reciprocal rank, latency, top score) so bad retrieval is debuggable.

## Acceptance Criteria
- **AC-1** A dataset of **≥ 20** questions loads and runs; the CLI reports `questionCount` and per-question pass/fail. *(Milestone 3 AC "≥20 test questions"; FR-EVAL-001.)*
- **AC-2** Hit@3 is computed and printed as a fraction in [0,1]. *(Milestone 3 AC; FR-EVAL-002.)*
- **AC-3** MRR is computed and printed. *(Milestone 3 AC; FR-EVAL-002.)*
- **AC-4** Hit@1, Hit@5, avg latency, empty-result count, and below-threshold count are all computed. *(FR-EVAL-002.)*
- **AC-5** A JSON report is written to the `--output` path containing metrics + per-question results + mode + topK + model + timestamp. *(Milestone 3 AC "saved to JSON"; FR-EVAL-002.)*
- **AC-6** `tovli eval ... --fail-below-hit-at-3 0.8` exits with a **non-zero** code when Hit@3 < 0.8 and **0** when Hit@3 ≥ 0.8. *(Milestone 3 AC "fail if below threshold"; FR-EVAL-003.)*
- **AC-7** Relevance is judged by matching returned results against `expectedChunkIds` (exact) and/or `expectedSourceFiles` (path-tolerant); a question with neither is rejected at load time. *(FR-EVAL-001 invariant; [ADR-0004](../../adrs/0004-relevance-judgment.md).)*
- **AC-8** On the curated `eval/questions.json` over the repo's own docs, **Hit@3 ≥ 0.80** with the ONNX embedder. *(Milestone 3 target quality. Verified locally with ONNX; CI uses deterministic mock-embedder + metric-math tests — [ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md).)*

## Constraints
- **C1** Rust edition 2021; MSVC toolchain for the ONNX path, pure-Rust subset for mock-only builds. **No Docker.**
- **C2** Local-first: eval uses the same local embedder as `ingest`/`search`; no network, no LLM (R2).
- **C3** Read-only: Evaluation only reads Retrieval output; it never writes the index or mutates embeddings. The only thing it writes is the report file.
- **C4** Performance (NFR §9.1): 50 questions evaluate in **< 2 minutes** without answer generation.
- **C5** Clippy-clean; `main.rs` `eval` handler delegates to `EvaluationService` (R9).
- **C6** Reuse the M2 `SearchService` + read adapters unchanged; no changes to the Retrieval domain.
- **C7** Tests live in `./tests`; ADRs in `./docs/adrs`. *(Project convention.)*

## Edge Cases
- **E1** Question with neither `expectedChunkIds` nor `expectedSourceFiles` → reject the dataset with a clear error (evaluation.md EvalQuestion invariant 1).
- **E2** Malformed / empty JSON dataset → clear parse error, non-zero exit, no partial report.
- **E3** A question returns **zero** results (empty match or empty index) → reciprocalRank 0, counts toward `emptyResultCount`; never a crash (E3 of retrieval feeds this).
- **E4** `expectedSourceFiles` path differs in form from the indexed `source_path` (`./docs/a.md` vs `docs\a.md` vs absolute) → **path-tolerant** matching (normalize separators; match by suffix/basename) — [ADR-0004](../../adrs/0004-relevance-judgment.md).
- **E5** `--top-k` < 5 → still fetch ≥ 5 so Hit@5 + MRR are computable; the requested `topK` is recorded — [ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md).
- **E6** Embedding-model mismatch during a search (AC-7 of M2) → **fatal**: abort the EvalRun with `status: "failed"` and the actionable message (don't silently report 0% — a config error must not look like a quality regression).
- **E7** Duplicate matches (a returned chunk matches both an expected chunk id and source file) → counts once; first relevant rank only.
- **E8** Hit@3 exactly equal to the threshold → **pass** (fail only when strictly below).
- **E9** Empty index (nothing ingested) → every question yields an empty RetrievalRun → metrics all 0; report still written; threshold gate may fail (correctly).

## Open Decisions (resolved in Architecture, recorded as ADRs)
- **D-RELEVANCE** → **[ADR-0004](../../adrs/0004-relevance-judgment.md)** A result is relevant iff its `chunk_id ∈ expectedChunkIds` OR its `source_path` path-tolerantly matches an `expectedSourceFiles` entry; source-file matching is the robust default because chunk ids are content-hash-derived and unstable across re-chunking.
- **D-DEPTH/CI** → **[ADR-0005](../../adrs/0005-eval-depth-and-ci-determinism.md)** Always retrieve `max(topK, 5)` so Hit@1/3/5 + MRR come from one run; CI determinism via pure metric-math unit tests + a mock-embedder exact-match integration test, with the semantic Hit@3 ≥ 0.80 target verified locally under ONNX.

## Phase-1 Gate (criteria for advancing to Pseudocode)
- [x] ≥ 3 acceptance criteria — **8** (AC-1…AC-8), each traced to a PRD FR / Milestone-3 AC.
- [x] Explicit constraints — **C1…C7**.
- [x] Edge cases identified — **E1…E9**, including the non-obvious ones (E4 path matching, E5 depth, E6 fatal mismatch).
- [x] Requirements trace to PRD §8.7 / §12.5 / Milestone 3 / §9.1.

**Gate result: PASS.** → Phase 2 (Pseudocode).
