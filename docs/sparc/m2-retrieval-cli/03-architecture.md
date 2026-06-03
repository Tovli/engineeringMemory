# SPARC Phase 3 — Architecture: M2 Retrieval CLI

Rust translation of the **Retrieval bounded context** (`docs/ddd/contexts/retrieval.md`), wired to
the M0/M1 stack: `ruvector-core` 2.2.0 (`VectorDB`, read side), `redb` (`documents.redb`, read side),
local ONNX/MiniLM or `MockEmbedder` (query embedding). Hexagonal, **read-only**: domain ← ports ←
application ← infra; the CLI calls application only (PRD §9.4). Key decisions: **[ADR-0001](../../adrs/0001-retrieval-bounded-context.md)**,
**[ADR-0002](../../adrs/0002-project-tag-filter-join.md)**, **[ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)**.

## Module layout
```
src/
  main.rs                       # add `Search(SearchArgs)` subcommand — arg parse only, delegates (C5/R9)
  lib.rs                        # add `pub mod retrieval;`
  ingestion/ ...                # unchanged (shared kernel: Embedder, EmbeddingModelVersion, DocumentId)
  retrieval/
    mod.rs
    domain/                     # pure types, no engine/infra crates
      query.rs                  #   Query, SearchMode, MetadataFilter
      retrieval_run.rs          #   RetrievalRun (aggregate root), RunReason (Ok | IndexEmpty)
      retrieval_result.rs       #   RetrievalResult (value object)
      explain.rs                #   ExplainPayload, ExplainResultDetail
      errors.rs                 #   RetrievalError (EmbeddingModelMismatch, EmptyQuery, InvalidTopK)
    ports.rs                    # VectorSearchPort, DocumentLookupPort, RawSearchResult, DocMeta
    application/
      search_service.rs         #   SearchService<E,S,L>: guard→embed→overfetch→join→filter→rank→explain
      filters.rs                #   pure filter predicates (project/tag/source/deleted) — unit-tested
      scoring.rs                #   distance→similarity normalization + threshold (ADR-0003)
    infra/
      ruvector_search.rs        #   RuVectorSearchAdapter (VectorSearchPort) over ruvector-core VectorDB
      redb_lookup.rs            #   RedbDocumentLookup (DocumentLookupPort) over documents.redb
tests/
  retrieval_search.rs           # integration tests (AC-1..AC-8) — see ./tests convention (C7)
```

## Dependency direction (acyclic, read-only)
```
                 ┌─────────────┐
 CLI (main.rs) ─▶│ application │─▶ ports ─▶ domain
                 └─────────────┘              ▲
 infrastructure ─(implements ports)───────────┘   depends on: ruvector-core (read), redb (read)
 shared kernel:  ingestion::{Embedder, EmbeddingModelVersion, DocumentId, ChunkId}
```
- `retrieval::domain` imports nothing project-internal except shared-kernel value objects; no engine crates.
- `retrieval::ports` references only domain + shared-kernel types.
- `retrieval::application` depends on ports + domain; injected concretes via generics (`<E,S,L>`).
- only `retrieval::infra` touches `ruvector-core` / `redb`. → **no cycles.** (Risk 1 contained.)

## Typed contracts (ports)
```rust
// --- domain ---
pub enum SearchMode { Vector }                      // Keyword/Hybrid added in M5
pub struct MetadataFilter { pub project: Option<String>, pub tags: Vec<String>, pub source: Option<String> }
impl MetadataFilter { pub fn is_empty(&self) -> bool { /* none set */ } }

pub struct Query {
    pub text: String, pub mode: SearchMode, pub filters: MetadataFilter,
    pub top_k: usize, pub embedding_model: EmbeddingModelVersion,   // shared-kernel VO
}
pub struct RetrievalResult {
    pub rank: usize, pub chunk_id: ChunkId, pub document_id: DocumentId,
    pub source_path: String, pub score: f32,                       // similarity [0,1] (ADR-0003)
    pub preview: String, pub heading_path: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}
pub enum RunReason { Ok, IndexEmpty }                              // AC-8
pub struct RetrievalRun {
    pub id: String, pub query: Query, pub results: Vec<RetrievalResult>,
    pub search_mode: SearchMode, pub top_k: usize, pub latency_ms: u128,
    pub below_threshold_count: usize, pub reason: RunReason,
    pub explain: Option<ExplainPayload>, pub completed_at: String,
}

// --- ports (the read-only seams) ---
pub struct RawSearchResult {                                       // ACL-internal shape
    pub chunk_id: String, pub document_id: String, pub source_path: String,
    pub distance: f32,                                             // raw cosine distance (ADR-0003)
    pub preview: String, pub heading_path: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}
pub struct DocMeta { pub project: Option<String>, pub tags: Vec<String>, pub source_path: String, pub status: DocumentStatus }

pub trait VectorSearchPort {                                       // ADR-0001
    fn vector_search(&self, query_vec: &[f32], k: usize) -> anyhow::Result<Vec<RawSearchResult>>;
}
pub trait DocumentLookupPort {                                     // ADR-0002
    fn find_many(&self, ids: &[DocumentId]) -> anyhow::Result<std::collections::HashMap<DocumentId, DocMeta>>;
    // The indexed model lives in documents.redb (every IngestionDocument records it), so the
    // guard reads it here — this also lets the CLI size the vector store by the indexed
    // dimension before opening it (avoids a dimension-mismatch at open time).
    fn indexed_model_version(&self) -> anyhow::Result<Option<EmbeddingModelVersion>>;
}
// query embedding reuses ingestion::ports::Embedder (shared kernel)
```
`SearchService` is generic — `SearchService<E: Embedder, S: VectorSearchPort, L: DocumentLookupPort>` —
so tests drive it with `MockEmbedder` + in-memory fake store/lookup (no ONNX, no disk), exactly like
M1's `IngestionOrchestrator`.

## How decisions land
| Decision | Resolution in this architecture |
|---|---|
| **D-CTX** ([ADR-0001](../../adrs/0001-retrieval-bounded-context.md)) | New `src/retrieval/` module; read-only `VectorSearchPort` + `DocumentLookupPort`; reuse `Embedder`/`EmbeddingModelVersion` from ingestion (shared kernel). |
| **D-FILTER** ([ADR-0002](../../adrs/0002-project-tag-filter-join.md)) | `RedbDocumentLookup` resolves candidate `document_id`s → `DocMeta`; `application/filters.rs` applies project (eq) / tags (multi-valued AND) / source (eq) and drops `Deleted` docs (E9). `--source` filter also matchable via `RawSearchResult.source_path`. |
| **D-SCORE** ([ADR-0003](../../adrs/0003-score-semantics-and-overfetch.md)) | `RuVectorSearchAdapter` returns raw `distance`; `application/scoring.rs` maps `similarity = clamp(1−distance,0,1)`; over-fetch `K×5` when filters set; `SIMILARITY_THRESHOLD = 0.30` → `below_threshold_count`. |
| **Model guard** (R6/AC-7) | `DocumentLookupPort::indexed_model_version()` reads the embedding model+dimension from `documents.redb` (every `IngestionDocument` records `embedding_model`/`embedding_dimension`); compared with `query.embedding_model` (name + dimension only — `created_at` ignored) before search. `None` ⇒ `IndexEmpty` (AC-8). The CLI uses the returned dimension to open the vector store, so it never opens with a wrong dimension. |

## Infra adapter notes
- **`RuVectorSearchAdapter`** opens the same `ruvector-core` `VectorDB` M1 wrote
  (`.tovli/vectors.redb`) with the same `DbOptions` (Cosine, dimension from the indexed model).
  `vector_search` builds `SearchQuery{ vector, k, filter: None, ef_search: None }` (filter stays
  `None` — filtering is app-side, ADR-0002), calls `db.search`, and maps each `SearchResult` →
  `RawSearchResult`, reading `document_id`/`source_path`/`preview`/`heading_path` from the metadata
  the M1 adapter stored (`heading_path` was joined with `" > "` at index time → split back).
- **`RedbDocumentLookup`** opens `.tovli/documents.redb` read-only and reads `IngestionDocument`
  records, projecting to `DocMeta`. Reuses the table layout from `RedbDocumentRepository` (keyed by
  source_path; an `id → record` scan or index supports `find_many` by `document_id`).
- Both adapters open their stores **read-only** in the `search` invocation (C3).

## Constraint coverage
- **C1/C6** reuses M0/M1 `ruvector-core` + `redb`; no Docker, no new engine, no migration.
- **C2** query embedding uses the same local `Embedder` builder as `ingest`; offline by default.
- **C3** Retrieval implements only read ports; no `upsert`/`delete`/`save` reachable from this context.
- **C4** HNSW knn `O(K·F·log N)` + `O(D)` redb reads ⇒ < 1 s @ 5k chunks (embedding dominates).
- **C5/R9** `main.rs` `Search` handler parses args, builds ports, calls `SearchService::search`, prints.
- **C7** integration tests in `./tests/retrieval_search.rs`; ADRs in `./docs/adrs/`.

## Domain events (logged, NFR §9.3 — no bus yet)
`SearchExecuted` on success (id, mode, top_k, latency_ms, n_results, filters, scores);
`SearchFailed` on `EmbeddingModelMismatch`. Consumed later by Evaluation (M3) and Feedback (M6).

## Phase-3 Gate (to advance → Refinement)
- [x] Architecture addresses all constraints (C1–C7 table above).
- [x] API contracts are typed (ports as Rust traits with explicit `RawSearchResult` / `DocMeta` / error types).
- [x] No circular dependencies (domain ← ports ← application ← infra; engine crates only in infra).
- [x] Every Phase-1 acceptance criterion has a home: AC-1/2 (search_service + formatter), AC-3 (over-fetch+trim), AC-4 (filters.rs + DocumentLookupPort), AC-5/E4 (RunReason/empty results), AC-6 (explain.rs), AC-7 (model guard), AC-8 (RunReason::IndexEmpty).
- [x] The three surfaced risks each have an owning ADR (0001/0002/0003).

**Gate result: PASS.** → Phase 4 (Refinement: implement + tests in `./tests`).
