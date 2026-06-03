# SPARC Phase 1 — Specification: M2 Retrieval CLI

Feature: **M2 — Retrieval CLI** · Slug: `m2-retrieval-cli` · Started: 2026-06-03
Derived from PRD §8.5 (FR-SRCH-*), §11.4–11.5 (Query / RetrievalResult), §12.3 (`search` CLI), §13 Milestone 2.
Bounded context: **Retrieval** (`docs/ddd/contexts/retrieval.md`) — read-only consumer of the index M1 populates.
Stack continues M0/M1: **Rust (edition 2021) + `ruvector-core` 2.2.0** (embedded `VectorDB`), `redb` sidecar, local ONNX/MiniLM (dim 384) or mock embedder.

## Goal
`tovli search "<question>"` embeds the query with the **same** model the index was built with,
runs a top-K vector similarity search over the chunks M1 stored, applies optional
`--project` / `--tag` / `--source` filters, and prints ranked results (rank, score, source file,
heading path, preview, chunk id). `--explain` adds a debugging payload. CLI stays thin (PRD §9.4).

## Scope (Milestone 2)
**In:** query embedding · top-K **vector** search · result formatting · metadata filters
(project / tag / source) · explain mode · embedding-model-compatibility guard.
**Out (later milestones):** keyword & hybrid modes and score fusion → **M5**; evaluation metrics → **M3**;
answer generation → **M4**; feedback → **M6**. The `--mode` flag exists now but only `vector` is
accepted in M2; `keyword`/`hybrid` return a clear "available in Milestone 5" error. The
`SearchStrategy` seam is designed so M5 plugs in without touching the domain.

## Requirements
- **R1** (FR-SRCH-001) `tovli search "<q>" --top-k <K>` embeds the query, searches RuVector, returns the top-K chunks; default `K = 8`.
- **R2** (FR-SRCH-001) Each result includes: source file, chunk id, document id, score, preview, heading path, rank.
- **R3** (FR-SRCH-002) Filter by `--project <p>`, `--tag <t>` (repeatable), `--source <path>`; filters are applied at query time and the active filters are echoed in the output.
- **R4** (FR-SRCH-002) Empty result sets are handled gracefully (clear "no results" message, exit 0, not a crash).
- **R5** (FR-SRCH-004) `--explain` shows: query embedding provider + dimension, search mode, filters applied, ranking method, per-result score, and why each chunk was eligible.
- **R6** (DDD invariant 4, PRD Risk 5 / FR-EMB-002) Before searching, verify the query embedding model matches the model the index was built with; on mismatch fail with an actionable error (run `tovli reembed`) — never silently mix dimensions.
- **R7** (PRD §11.4) A `Query` value object carries `questionText`, `searchMode`, `filters`, `topK`, optional `embeddingModel`; `questionText` non-empty, `topK` positive.
- **R8** (PRD §11.5 / retrieval.md) A `RetrievalRun` aggregates ordered `RetrievalResult`s (rank 1 = best), records `latencyMs` and `belowThresholdCount`; immutable once built.
- **R9** (PRD §9.4) CLI handler contains **no** retrieval logic; it parses args and delegates to a `SearchService` in the application layer.
- **R10** (PRD §7, §15 Risk 1) All RuVector access stays behind a read-only port (ACL); the domain never imports `ruvector-core`.

## Acceptance Criteria
- **AC-1** `tovli search "architecture layering rules"` over an ingested corpus returns ranked chunks, best match first, each line showing rank, score, source file, and a preview. *(Use Case 2, Milestone 2 AC "search by natural language" + "ranked chunks".)*
- **AC-2** Every result line includes the **source file** and a **similarity score**; scores are non-increasing down the list (rank 1 has the highest score). *(Milestone 2 AC "results include source file and score".)*
- **AC-3** `--top-k K` returns at most `K` results; when fewer eligible chunks exist, it returns all of them without error.
- **AC-4** `--project <p>` returns only chunks from documents tagged with that project; `--tag <t>` only chunks whose document carries that tag; `--source <path>` only chunks from that source file. Combined filters AND together. *(Milestone 2 AC "filters work by project/tag/source".)*
- **AC-5** A query matching nothing (after filters) prints a clear "no results" message and exits 0. *(R4.)*
- **AC-6** `--explain` prints the query embedding provider + dimension, the search mode, the active filters, the ranking method (`cosine`), and a per-result eligibility/score breakdown. *(Milestone 2 AC "explain mode shows ranking details".)*
- **AC-7** Searching an index built with a different embedding model/dimension than the active query embedder fails with a clear error naming both models and suggesting `tovli reembed` — and writes nothing. *(R6.)*
- **AC-8** Searching an empty index (nothing ingested) prints "index is empty — run `tovli ingest` first" and exits 0, not a panic.

## Constraints
- **C1** Rust edition 2021; builds under the MSVC toolchain established in M1 (ONNX path) and the pure-Rust subset for mock-only builds. **No Docker.**
- **C2** Local-first / privacy (NFR §9.2): query embedding uses the **same local provider** as ingestion by default; no data leaves the machine.
- **C3** Read-only: the Retrieval context **never** writes to the vector store or the document repo (upstream-supplier relationship with Ingestion). It opens both stores read-only.
- **C4** Performance (NFR §9.1): search over 5,000 chunks returns in **< 1 second** locally (HNSW; embedding the single query string dominates for ONNX).
- **C5** Clippy-clean; `main.rs` `search` handler delegates to `SearchService` (no engine/SQL/ranking logic in the handler, PRD §9.4).
- **C6** Reuse the M0/M1 `ruvector-core` `VectorDB` and the `redb` document sidecar — no new storage engine, no schema migration.
- **C7** Tests live in `./tests` (integration) with unit tests inline per module; ADRs live in `./docs/adrs`. *(Project convention.)*

## Edge Cases
- **E1** Empty / whitespace-only query string → reject before embedding with "query must not be empty" (R7).
- **E2** `--top-k 0` or negative → reject with "top-k must be a positive integer".
- **E3** Empty index (no chunks) → AC-8 message, exit 0.
- **E4** Filter matches zero chunks → AC-5 "no results", exit 0 (distinct from "index empty").
- **E5** **Post-filter truncation:** `ruvector-core`'s `VectorDB::search` applies its metadata filter via `results.retain` **after** the HNSW top-K cut, so a naive `k = topK` with a filter can yield far fewer than `topK` hits. → must **over-fetch** candidates then filter then trim (see Architecture / [ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)).
- **E6** **Filter field gap:** ingestion writes `source_path`/`title` into chunk vector metadata but **not** `project`/`tags` (those live on `IngestionDocument` in `documents.redb`); tags are multi-valued and `ruvector-core`'s filter only does exact equality. → project/tag filtering resolves through a read-only document-lookup join (see [ADR-0002](../../adrs/0002-project-tag-filter-join.md)).
- **E7** **Score semantics:** `ruvector-core` returns cosine **distance** (lower = closer) but the DDD `RetrievalResult.score` contract is `[0,1]`, higher = better. → normalize `similarity = 1 − distance` for the domain and display ([ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)).
- **E8** Model/dimension mismatch between query embedder and index → AC-7 error before any search.
- **E9** A retrieved chunk whose owning document was soft-deleted since indexing → exclude from results (read `status` via the document lookup) so deleted docs don't surface.
- **E10** Below-threshold results: results with `similarity < SimilarityThreshold` are still returned in M2 (no answer-gating yet) but counted in `belowThresholdCount` and flagged in `--explain`; the threshold feeds M3/M4.

## Open Decisions (resolved in Architecture, recorded as ADRs)
- **D-CTX** → **[ADR-0001](../../adrs/0001-retrieval-bounded-context.md)** Retrieval gets its own `src/retrieval/` hexagonal module (domain ← ports ← application ← infra) with a read-only `VectorSearchPort`, rather than extending the ingestion `VectorStorePort`.
- **D-FILTER** → **[ADR-0002](../../adrs/0002-project-tag-filter-join.md)** project/tag/source filtering is applied in the application layer over candidates resolved through a read-only `DocumentLookupPort` (join against `documents.redb`), not pushed into `ruvector-core`'s exact-equality post-filter.
- **D-SCORE** → **[ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)** report `similarity = 1 − cosine_distance`; over-fetch `topK × OVERFETCH` candidates before app-level filtering/trim to defeat post-filter truncation.

## Phase-1 Gate (criteria for advancing to Pseudocode)
- [x] ≥ 3 acceptance criteria — **8** (AC-1…AC-8), each traced to a PRD FR / Milestone-2 AC.
- [x] Explicit constraints — **C1…C7**.
- [x] Edge cases identified — **E1…E10**, including the three non-obvious ones surfaced from the M1 code (E5 post-filter, E6 filter-field gap, E7 score semantics).
- [x] Requirements trace to PRD §8.5 / §11 / §12.3 / Milestone 2.

**Gate result: PASS.** → Phase 2 (Pseudocode).
