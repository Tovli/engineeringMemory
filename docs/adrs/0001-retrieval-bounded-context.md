# ADR-0001: Retrieval as its own read-only hexagonal module

- **Status:** Accepted
- **Date:** 2026-06-03
- **Milestone:** M2 — Retrieval CLI
- **Context refs:** `docs/ddd/contexts/retrieval.md`, PRD §8.5, §10, §15 Risk 1

## Context
M1 built the Ingestion context under `src/ingestion/` (domain ← ports ← application ← infra).
M2 adds search. The DDD model treats **Retrieval** as a *separate bounded context* with an
**upstream-supplier** relationship to Ingestion: it reads the index Ingestion populates and must
**never** write to it. The existing ingestion `VectorStorePort` (`src/ingestion/ports.rs`) is a
**write** seam (`upsert_chunks`, `delete_by_document`) — it has no search method, and the M1
architecture note that claimed an `indexed_model_version()` method was never implemented.

Options considered:
1. **Extend the ingestion `VectorStorePort`** with `vector_search` + `indexed_model_version`.
2. **Create a separate `src/retrieval/` module** with its own read-only ports.

## Decision
Create **`src/retrieval/`** as an independent hexagonal module mirroring `src/ingestion/`:

```
src/retrieval/
  domain/        query.rs, retrieval_run.rs, retrieval_result.rs, explain.rs, errors.rs   (pure)
  ports.rs       VectorSearchPort, DocumentLookupPort, (re-uses ingestion::Embedder)
  application/   search_service.rs   (generic over the ports; SearchStrategy seam for M5)
  infra/         ruvector_search.rs (VectorSearchPort), redb_lookup.rs (DocumentLookupPort)
```

- `VectorSearchPort` is **read-only**: `vector_search(qvec, k) -> Vec<RawSearchResult>` and
  `indexed_model_version() -> Option<EmbeddingModelVersion>`.
- `DocumentLookupPort` is **read-only**: `find_many(&[DocumentId]) -> Map<DocumentId, DocMeta>`
  where `DocMeta = { project, tags, source_path, status }`.
- The query embedder reuses the existing `ingestion::ports::Embedder` trait (shared kernel) so
  query-time and index-time embeddings come from the same provider abstraction (C2).
- `EmbeddingModelVersion` is reused from `ingestion::domain` (shared kernel value object).

## Consequences
- **+** Read/write separation is enforced by the type system — Retrieval physically cannot write.
- **+** Mirrors M1's structure; same testing pattern (generic service + in-memory fakes + MockEmbedder).
- **+** Risk 1 (RuVector instability) stays contained: only `retrieval/infra/ruvector_search.rs`
  touches `ruvector-core` on the read side.
- **−** Two adapters now open the same `ruvector-core` `VectorDB` (write side in ingestion, read
  side in retrieval). Acceptable: opened read-only in distinct CLI invocations; no shared process state.
- **−** Some value-object duplication risk with Ingestion; mitigated by reusing the shared-kernel
  types (`EmbeddingModelVersion`, `DocumentId`, `ChunkId`) rather than redefining them.
- Adds `pub mod retrieval;` to `src/lib.rs`.
