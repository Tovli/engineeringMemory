# SPARC Phase 3 — Architecture: M1 Document Ingestion

Rust translation of the **Ingestion bounded context** (`docs/ddd/contexts/ingestion.md`),
wired to the stack resolved in Phase 1–2: `ruvector-core` (vectors), `redb` (document/run
records), local ONNX/MiniLM (embeddings). Hexagonal: domain ← ports ← application ←
infrastructure; the CLI calls application only.

## Module layout
```
src/
  main.rs                 # `tovli` CLI entry — arg parse only, delegates to application (C5)
  bin/verify-onnx.rs      # task #13 ONNX verifier (feature = onnx)
  vector_store.rs         # M0 seam → basis for the VectorStorePort adapter
  ingestion/
    domain/               # pure types, no external crates
      document.rs         #   IngestionDocument (aggregate root), DocumentStatus
      chunk.rs            #   Chunk (entity), ChunkEmbedding (value object)
      run.rs              #   IngestionRun (aggregate root), IngestionError
      config.rs           #   ChunkingConfig, EmbeddingModelVersion (value objects)
      events.rs           #   DocumentIngested, ChunksCreated, EmbeddingsGenerated, ChunksIndexed, ...
    ports.rs              # traits (the seams) — depend only on domain
    application/
      orchestrator.rs     #   IngestionOrchestrator: scan→parse→dedup→chunk→embed→persist→emit
      chunking.rs         #   ChunkingService (markdown-aware)
      embedding.rs        #   EmbeddingService (dim-guard wrapper over Embedder port)
    infra/
      parsers/            #   MarkdownParser, PlainTextParser, JsonParser, YamlParser  (FileParser)
      onnx_embedder.rs    #   OnnxEmbedder  (Embedder)  — wraps ruvector-core OnnxEmbedding
      mock_embedder.rs    #   MockEmbedder  (Embedder)  — deterministic, tests (FR-EMB-001)
      redb_repo.rs        #   RedbDocumentRepository + RedbRunRepository  (D-PERSIST)
      ruvector_store.rs   #   RuVectorStoreAdapter  (VectorStorePort) — extends M0 RuVectorStore
```

## Dependency direction (acyclic)
```
                 ┌─────────────┐
 CLI (main.rs) ─▶│ application │─▶ ports ─▶ domain
                 └─────────────┘              ▲
 infrastructure ─(implements ports)───────────┘   depends on: ruvector-core, ort, redb, blake3
```
- `domain` imports nothing project-internal and no engine crates (pure, unit-testable).
- `ports` references only `domain` types.
- `application` depends on `ports` + `domain`; never on `infra` concretes (injected via generics/`dyn`).
- `infra` implements `ports`; only `infra` touches `ruvector-core`/`ort`/`redb`. → **no cycles.**

## Typed contracts (ports)
```rust
// --- domain value objects ---
pub struct EmbeddingModelVersion { pub name: String, pub dimension: usize, pub created_at: String }
pub struct ChunkingConfig { pub target_tokens: u32, pub max_tokens: u32, pub overlap_tokens: u32 } // invariant: overlap<target<max

pub struct ParsedDoc { pub text: String, pub title: Option<String> }
pub struct ChunkWithEmbedding<'a> { pub chunk: &'a Chunk, pub vector: Vec<f32> }

// --- ports (traits) ---
pub trait FileParser {
    fn extensions(&self) -> &'static [&'static str];
    fn parse(&self, raw: &[u8]) -> Result<ParsedDoc, ParseError>;   // Err on non-UTF8/binary (E3)
}
pub trait Embedder {
    fn model_version(&self) -> &EmbeddingModelVersion;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>; // len == dimension
}
pub trait VectorStorePort {
    fn upsert_chunks(&self, items: &[ChunkWithEmbedding]) -> Result<(), StoreError>;
    fn delete_by_document(&self, id: &DocumentId) -> Result<(), StoreError>;
    fn indexed_model_version(&self) -> Result<Option<EmbeddingModelVersion>, StoreError>; // AC-8 guard
}
pub trait DocumentRepository {
    fn find_by_path(&self, path: &str) -> Result<Option<IngestionDocument>, RepoError>;
    fn save(&self, doc: &IngestionDocument) -> Result<(), RepoError>;
    fn soft_delete(&self, id: &DocumentId) -> Result<(), RepoError>;
    fn all_active_under(&self, root: &str) -> Result<Vec<IngestionDocument>, RepoError>;
}
pub trait RunRepository { fn save(&self, run: &IngestionRun) -> Result<(), RepoError>; }
```
The `IngestionOrchestrator` is generic over these traits (`<P: FileParser, E: Embedder,
S: VectorStorePort, D: DocumentRepository, R: RunRepository>`), so the mock embedder and an
in-memory store make the whole pipeline unit-testable without ONNX or disk.

## How decisions land
| Decision | Resolution in this architecture |
|---|---|
| **D-EMB** | `OnnxEmbedder` (infra) wraps `ruvector_core::OnnxEmbedding` (MiniLM, dim 384) and implements `Embedder`. **Use `OnnxEmbedding::from_files(model, tokenizer, id)` with a locally-cached model** (`models/all-MiniLM-L6-v2/`), NOT `from_pretrained` (hf-hub 0.3 hits `RelativeUrlWithoutBase`). `MockEmbedder` implements the same port for tests. → constraint **C7** (MSVC) applies; verified end-to-end by task #13 (load 252ms, embed 38ms, dog~cat 0.66 > dog~car 0.48). |
| **D-HASH** | `blake3` for both file `contentHash` and per-chunk `contentHash` (C6, idempotency). |
| **D-TOK** | `tokenCount` via char/word heuristic in `ChunkingService`; real tokenizer deferred (OnnxEmbedding truncates at 512 internally). |
| **D-PERSIST** | `redb` sidecar holds `IngestionDocument` + `Chunk` records + `IngestionRun` (the DDD repositories). `ruvector-core` holds vectors keyed by `chunk_id` with minimal metadata. Two stores, one transaction boundary per document. |

## Constraint coverage
- **C1** reuses M0 `VectorStore` seam (now `VectorStorePort`/`RuVectorStoreAdapter`); no Docker.
- **C2** default `Embedder` is local ONNX — offline after first model download; no data leaves the machine.
- **C3/C7** ONNX needs MSVC (settled); pure-Rust subset otherwise.
- **C4** batch embedding + hash-skip of unchanged files keeps 100 md files < 5 min.
- **C5** `main.rs` only parses args and calls `IngestionOrchestrator`.
- **C6** blake3 hashing + deterministic chunk IDs (`blake3(documentId + chunkIndex + contentHash)`).

## Domain events (emitted by orchestrator, consumed later by Retrieval)
`DocumentIngested`, `DocumentDeleted`, `ChunksCreated`, `EmbeddingsGenerated`, `ChunksIndexed`,
`IngestionRunCompleted` — see `docs/ddd/domain-events.md`. For M1 these are logged (structured
JSON, NFR §9.3); a real event bus is out of scope.

## Phase-3 Gate (to advance → Refinement)
- [x] Architecture addresses all constraints (C1–C7 table above)
- [x] API contracts are typed (ports as Rust traits with explicit error types)
- [x] No circular dependencies (domain ← ports ← application ← infrastructure)
- [ ] **Contingent:** ONNX path verified end-to-end (task #13 running) before implementing the `OnnxEmbedder`
