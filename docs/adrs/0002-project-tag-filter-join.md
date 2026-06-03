# ADR-0002: Apply project/tag/source filters via a document-lookup join, not the vector store

- **Status:** Accepted
- **Date:** 2026-06-03
- **Milestone:** M2 — Retrieval CLI
- **Context refs:** PRD §8.5 FR-SRCH-002, `src/ingestion/orchestrator.rs`, `src/ingestion/infra/ruvector_store.rs`, `ruvector-core` `vector_db.rs::search`

## Context
M2 must filter results by `--project`, `--tag`, `--source` (AC-4). Two facts from the M1 code
constrain how:

1. **The vector store doesn't carry project/tags.** Ingestion writes `source_path`, `title`,
   `embedding_model`, `embedding_dimension`, `char_length`, `document_id`, `chunk_index`,
   `heading_path`, `content_hash`, `token_count`, `preview` into each chunk's RuVector metadata
   (`orchestrator.rs:107-116`, `ruvector_store.rs:58-67`). It does **not** write `project` or
   `tags` — those live only on `IngestionDocument` in `documents.redb`.
2. **`ruvector-core`'s filter is exact-equality post-filtering.** `VectorDB::search` does
   `results.retain(|r| filter.iter().all(|(k,v)| metadata.get(k) == Some(v)))` *after* the
   top-K HNSW cut. Tags are **multi-valued**, so exact-equality can't match "one tag among many",
   and post-filtering shrinks an already-truncated result set (see ADR-0003).

Options:
1. **Backfill** `project`/`tags` into chunk vector metadata at ingest time and push filters into
   `ruvector-core`.
2. **Join** at query time: fetch candidates from the vector store, then filter in the application
   layer using document metadata read from `documents.redb` via a read-only `DocumentLookupPort`.

## Decision
Use **option 2 — the document-lookup join.** `SearchService` resolves the distinct `document_id`s
of the candidate hits through `DocumentLookupPort.find_many(...)` and applies all three filters in
Rust:

- `project`  → `doc.project == filter.project`
- `tags`     → every requested tag is contained in `doc.tags` (multi-valued **AND**)
- `source`   → `hit.source_path == filter.source` (uses the vector metadata directly)

The same join also drops hits whose owning document is `status == Deleted` (edge case E9).

## Consequences
- **+** Correct multi-valued tag semantics and combinable filters, which `ruvector-core`'s
  exact-equality filter cannot express.
- **+** No re-ingestion required; uses `documents.redb` as the authoritative source of project/tags.
- **+** Filtering logic is pure and unit-testable in the application layer, independent of RuVector.
- **+** Naturally excludes soft-deleted documents (E9) — the vector store may still hold their
  vectors until the next ingest's deletion pass.
- **−** Requires **over-fetching** candidates so enough survive filtering (paired with ADR-0003).
- **−** Couples Retrieval to the document store for reads. Contained behind `DocumentLookupPort`
  (an ACL); the domain stays ignorant of redb.
- **−** `O(D)` extra point reads per query (`D` = distinct candidate documents) — negligible at M2 scale.
- **Future:** if profiling ever shows the join is hot, revisit backfilling project/tags into chunk
  metadata as an optimization; this ADR would be superseded, not the port contract.
