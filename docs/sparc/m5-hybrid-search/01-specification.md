# SPARC Phase 1 - Specification: M5 Hybrid Search

Feature: **M5 - Hybrid Search** · Slug: `m5-hybrid-search` · Started: 2026-06-09
Derived from PRD §8.5 FR-SRCH-003/004, §8.7 FR-EVAL-002/003, §13 Milestone 5, and
[ADR-0009](../../adrs/0009-hybrid-search-rrf.md).

## Goal
Implement `--mode vector|keyword|hybrid` across `search`, `eval`, and `ask`. Keyword mode must search
full chunk content, not previews. Hybrid mode must combine vector and keyword candidate ranks with
Reciprocal Rank Fusion (RRF), preserve the existing metadata-filter semantics, and expose enough
`--explain` detail to debug why a result ranked.

## Scope
**In:** keyword index persisted during ingestion; keyword read port; keyword mode; hybrid RRF mode;
mode parsing for search/eval/ask; evaluation by mode; explain details for vector, keyword, and fused
scores.

**Out:** provider-native RuVector sparse/hybrid search; automatic mode comparison in one command;
feedback-driven learning; changing the answer-generation contract.

## Requirements
- **R1** `SearchMode` supports `Vector`, `Keyword`, and `Hybrid` with stable CLI strings.
- **R2** Ingestion writes a local deterministic keyword index containing full chunk content and deletes
  keyword entries when a document is reindexed or removed.
- **R3** Keyword mode returns ranked chunks by lexical score and keeps `RetrievalResult.score` in `[0,1]`.
- **R4** Hybrid mode independently fetches vector and keyword candidates, unions by `chunk_id`, and ranks
  by RRF using ADR-0009 constants.
- **R5** All modes share the existing project/tag/source filter and soft-delete behavior from ADR-0002.
- **R6** `--explain` records vector score, keyword score, fused score, ranking method, and eligibility
  reason for each emitted result.
- **R7** `eval --mode keyword|hybrid` runs through the same `EvaluationService` and writes normal reports.
- **R8** `ask --mode keyword|hybrid` retrieves through the selected mode and passes the resulting
  `RetrievalRun` to RAG unchanged.

## Acceptance Criteria
- **AC-1** `tovli search "zipDeploy 403" --mode keyword` can return a chunk whose exact terms are in the
  full chunk content.
- **AC-2** `tovli search "..." --mode hybrid --explain` shows ranking method `rrf` and non-empty
  per-result fused scores.
- **AC-3** Hybrid mode can rank a chunk higher when it appears in both vector and keyword candidate lists.
- **AC-4** Project/tag/source filters behave identically across vector, keyword, and hybrid modes.
- **AC-5** Modified/deleted documents do not leave stale keyword results.
- **AC-6** `tovli eval ./eval/questions.json --mode keyword|hybrid` is accepted and produces a report.
- **AC-7** Invalid modes fail with exit code 2 and a clear accepted-values message.

## Constraints
- **C1** Keep Retrieval read-only; keyword index writes happen only in Ingestion.
- **C2** Keep the domain independent of `redb`, `ruvector-core`, and lexical implementation details.
- **C3** No new external service. The default keyword index is local and deterministic.
- **C4** Preserve existing vector behavior and tests.
- **C5** Use TDD for production code changes.

## Edge Cases
- **E1** Blank keyword query yields the same empty-query validation as vector mode.
- **E2** Keyword index missing but documents/index empty returns an empty run, not a panic.
- **E3** Hybrid candidate exists in only one mode; missing rank contributes `0` to RRF.
- **E4** Equal fused scores use deterministic tie-breaking.
- **E5** Keyword mode does not need query embeddings, so embedding-model mismatch is fatal only for
  vector and hybrid modes.

## Phase-1 Gate
- [x] Requirements trace to ADR-0009 and PRD M5.
- [x] Acceptance criteria cover keyword, hybrid, filters, eval, ask, and stale index behavior.
- [x] Constraints preserve existing bounded-context boundaries.

**Gate result: PASS.** -> Phase 2 (Pseudocode).
