# ADR-0005: Fixed eval retrieval depth (≥5) and deterministic CI without ONNX

- **Status:** Accepted
- **Date:** 2026-06-03
- **Milestone:** M3 — Retrieval Evaluation
- **Context refs:** PRD §8.7 FR-EVAL-002, §12.5 (`--top-k 3`), §13 Milestone 3 (Hit@3 ≥ 0.80), `src/ingestion/infra/mock_embedder.rs`, ADR-0003

## Context
Two tensions in evaluating retrieval quality:

1. **Depth vs. the metric set.** The CLI exposes `--top-k` (PRD §12.5 shows `--top-k 3`), but the
   required metrics include **Hit@5** and **MRR**, which need at least the top 5 ranked results.
   If we retrieved only `top_k = 3`, Hit@5 would be uncomputable and MRR truncated.
2. **CI determinism vs. the quality target.** The Milestone-3 target (Hit@3 ≥ 0.80) is only meaningful
   with the **semantic** ONNX embedder, which is feature-gated (MSVC + `ort`) and too heavy/slow for
   the default `cargo test`. The deterministic `MockEmbedder` is non-semantic (blake3-derived), so it
   cannot demonstrate semantic quality — but it *is* perfectly reproducible.

## Decision
- **Retrieve `K = max(top_k, 5)`** for every evaluation query, regardless of the requested `top_k`.
  Hit@1/3/5 and MRR are all derived from this single ranked list; the requested `top_k` is still
  recorded in the EvalRun/report for provenance.
- **Split the quality bar from CI:**
  - *CI / `cargo test`* proves the **metric math and plumbing** are correct, deterministically:
    pure unit tests for `compute_metrics`, `is_relevant`, and `threshold_status` (canned inputs), plus
    an integration test that ingests a tiny corpus with the `MockEmbedder` and a crafted dataset whose
    question text **equals** a chunk's content — the mock embeds them identically (distance ≈ 0), so
    that chunk ranks #1 and Hit@1 = 1.0 deterministically.
  - *The semantic Hit@3 ≥ 0.80 target (AC-8)* is verified **locally/manually** by running
    `tovli ingest ./docs && tovli eval ./eval/questions.json` with the ONNX embedder, against the
    curated `eval/questions.json`. It is documented, not gated in CI.
- Reuse `retrieval::application::scoring::SIMILARITY_THRESHOLD` for `belowThresholdCount` so eval and
  search agree on "below threshold" (single source of truth; see ADR-0003).

## Consequences
- **+** All five metric families are always computable from one search per question.
- **+** `cargo test` stays fast, offline, and deterministic — no ONNX, no network, no flaky semantics.
- **+** The quality target is honest: it's measured with the real embedder, where it's meaningful.
- **−** A green CI does **not** by itself prove Hit@3 ≥ 0.80 — that requires the documented local ONNX
  run. The completion doc states this explicitly so the gap is visible, not implied.
- **−** `K = max(top_k, 5)` means a user asking for `--top-k 1` still pays for 5 neighbours. Negligible
  at M3 scale and necessary for Hit@5.
- **Future (M5):** when keyword/hybrid modes land, the same EvalRun/report compares modes side by side
  (FR-EVAL-002 "compare multiple search modes") with no change to the metric core.
