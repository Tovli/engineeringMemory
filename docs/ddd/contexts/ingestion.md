# Ingestion Bounded Context

> Source: `docs/prd.md` §§8.1–8.4, §10, §11.1–11.3, §15.

---

## Purpose and Responsibility

The Ingestion context owns the complete pipeline from raw file on disk to
indexed vector in RuVector. It is responsible for:

- Scanning a local folder for supported file types.
- Parsing raw file content into structured text.
- Splitting text into Chunks according to a ChunkingConfig.
- Generating an Embedding for each Chunk via an external embedding provider.
- Persisting Documents, Chunks, and Embeddings in the vector store.
- Enforcing idempotency: unchanged files and chunks must not be duplicated.
- Detecting modified files and triggering re-indexing of only changed content.
- Soft-deleting Chunks and Embeddings for removed source files.
- Publishing domain events so downstream contexts (Retrieval) know when
  the index has changed.

The Ingestion context does NOT perform retrieval, answer generation, or
evaluation. It does not depend on the Retrieval context.

---

## Aggregates

### `IngestionDocument` (Aggregate Root)

The aggregate root for a single source file's lifecycle in the system.
It owns all Chunks and Embeddings derived from that file.

```typescript
// Aggregate root
type IngestionDocument = {
  id: DocumentId;
  sourcePath: string;
  fileName: string;
  fileExtension: string;
  contentHash: ContentHash;
  title?: string;
  project?: string;
  tags: string[];
  status: DocumentStatus;
  chunkingConfig: ChunkingConfig;     // config active when last ingested
  embeddingModelVersion: EmbeddingModelVersion; // model used for embeddings
  chunks: Chunk[];                    // owned entities
  createdAt: string;                  // ISO-8601
  updatedAt: string;
  deletedAt?: string;
};

type DocumentStatus = "active" | "modified" | "deleted";
```

**Invariants:**
1. `contentHash` uniquely identifies the file content. Two IngestionDocuments
   with the same `sourcePath` and `contentHash` must not coexist as active.
   (Idempotent ingestion — PRD §8.1 FR-ING-003.)
2. All Chunks owned by a document must share the same `embeddingModelVersion`.
   Mixing model versions within one document's chunks is prohibited.
   (PRD §8.3 FR-EMB-001, Risk 5.)
3. A `deleted` document may retain its Chunks in a soft-deleted state; it
   must not appear in search results.
4. Re-ingesting a document with a new `contentHash` increments the document
   to `modified` status, invalidates old chunks, and triggers re-chunking.

---

### Chunk (Entity, owned by IngestionDocument)

```typescript
type Chunk = {
  id: ChunkId;
  documentId: DocumentId;
  chunkIndex: number;
  headingPath: string[];
  content: string;
  preview: string;
  contentHash: ContentHash;
  tokenCount: number;
  metadata: Record<string, string | number | boolean>;
  embedding?: ChunkEmbedding;  // set after EmbeddingService runs
  createdAt: string;
  updatedAt: string;
};
```

**Invariants:**
1. `chunkIndex` is zero-based and unique within its IngestionDocument.
2. `tokenCount` must not exceed `ChunkingConfig.maxChunkTokens`.
3. A Chunk may not split a fenced code block (```` ``` ````) across boundaries
   (PRD §8.2 FR-CHK-001, Risk 4).
4. `preview` is derived deterministically from `content` — it never requires
   an LLM call.
5. `contentHash` enables per-chunk change detection: if the chunk hash is
   unchanged, the chunk is skipped during re-ingestion.

---

### ChunkEmbedding (Value Object, owned by Chunk)

```typescript
type ChunkEmbedding = {
  id: string;               // storage-layer id, e.g. "emb_..."
  chunkId: ChunkId;
  modelVersion: EmbeddingModelVersion;
  vector: number[];
  createdAt: string;
};
```

**Invariants:**
1. `vector.length` must equal `modelVersion.dimension`.
2. A Chunk may hold at most one active ChunkEmbedding. When the embedding
   model is changed (ReEmbedCommand), the old embedding is replaced.

---

### IngestionRun (Aggregate Root)

Represents one execution of `tovli ingest`. Tracks progress and results.

```typescript
type IngestionRun = {
  id: string;            // "run_..."
  folderPath: string;
  chunkingConfig: ChunkingConfig;
  embeddingModelVersion: EmbeddingModelVersion;
  status: IngestionRunStatus;
  filesScanned: number;
  filesIngested: number;
  filesSkipped: number;   // unchanged hash
  filesErrored: number;
  chunksCreated: number;
  chunksSkipped: number;
  embeddingsGenerated: number;
  errors: IngestionError[];
  startedAt: string;
  completedAt?: string;
};

type IngestionRunStatus = "running" | "completed" | "failed";

type IngestionError = {
  sourcePath: string;
  reason: string;
};
```

**Invariants:**
1. A failed file ingestion does not abort the entire run unless configured
   (PRD §9.5). `filesErrored` is incremented instead.
2. `completedAt` is only set when `status` is `"completed"` or `"failed"`.

---

## Value Objects

### ChunkingConfig

```typescript
type ChunkingConfig = {
  readonly targetChunkTokens: number; // default 500
  readonly maxChunkTokens: number;    // default 800
  readonly overlapTokens: number;     // default 80
};
```

Invariants:
- `overlapTokens` < `targetChunkTokens` < `maxChunkTokens`.
- All values must be positive integers.

---

## Domain Events

| Event                   | Trigger                                          |
|-------------------------|--------------------------------------------------|
| `DocumentIngested`      | A Document is created or updated (hash changed)  |
| `DocumentDeleted`       | A source file is no longer present on disk       |
| `ChunksCreated`         | Chunking completes for a Document                |
| `EmbeddingsGenerated`   | Embeddings are stored for all Chunks of a Document |
| `ChunksIndexed`         | All Chunks and Embeddings for an IngestionRun are persisted; Retrieval may query |
| `IngestionRunCompleted` | An IngestionRun finishes (success or partial failure) |

Event payloads are in [`../domain-events.md`](../domain-events.md).

---

## Repository Interfaces

```typescript
interface IngestionDocumentRepository {
  findBySourcePath(sourcePath: string): Promise<IngestionDocument | null>;
  findByContentHash(hash: ContentHash): Promise<IngestionDocument | null>;
  save(doc: IngestionDocument): Promise<void>;
  softDelete(id: DocumentId): Promise<void>;
  findAllActive(): Promise<IngestionDocument[]>;
}

interface IngestionRunRepository {
  save(run: IngestionRun): Promise<void>;
  findById(id: string): Promise<IngestionRun | null>;
  findLatest(limit: number): Promise<IngestionRun[]>;
}
```

---

## Domain Services

### `FileParserService`
Responsibility: convert raw file bytes into plain text, respecting file type.
Input: `{ filePath: string; content: Buffer }`.
Output: `{ text: string; title?: string }`.
Delegates to file-type-specific parsers (Markdown, plain text, JSON, YAML).
This service lives in the domain — it does not call external APIs. Parser
implementations for each file type are behind a `FileParserPort` interface
so new types can be added without changing the domain.

### `ChunkingService`
Responsibility: split document text into Chunks according to a ChunkingConfig.
Input: `{ text: string; documentId: DocumentId; config: ChunkingConfig }`.
Output: `Chunk[]` (without embeddings).
Markdown-aware: preserves heading paths, avoids splitting code blocks.

### `EmbeddingService` (Domain Service, depends on EmbeddingPort ACL)
Responsibility: generate a ChunkEmbedding for each Chunk by calling the
embedding provider through the `EmbeddingPort` ACL.
Input: `{ chunks: Chunk[]; modelVersion: EmbeddingModelVersion }`.
Output: `ChunkEmbedding[]`.
Ensures `vector.length === modelVersion.dimension` for each result.

### `IngestionOrchestrator` (Application-level Domain Service)
Responsibility: coordinate the full ingestion pipeline for one file or folder:
scan → parse → deduplicate (hash check) → chunk → embed → persist → publish events.
This is the entry point called by the CLI `ingest` command.

---

## Anti-Corruption Layer

### `VectorStorePort` (ACL — wraps RuVector/Postgres)

```typescript
interface VectorStorePort {
  upsertChunks(
    chunks: ChunkWithEmbedding[],
    modelVersion: EmbeddingModelVersion
  ): Promise<void>;
  deleteChunksByDocumentId(documentId: DocumentId): Promise<void>;
  getIndexedModelVersion(): Promise<EmbeddingModelVersion | null>;
}
```

The domain calls `upsertChunks`. The ACL implementation translates this into
the appropriate RuVector/Postgres SQL (`INSERT ... ON CONFLICT DO UPDATE` with
pgvector or RuVector-specific DDL). The domain never sees `pg.Pool`, `knex`,
or RuVector client types.

**Why this matters:** PRD Risk 1 — RuVector APIs may be unstable. Isolating
behind this port means swapping the vector store only requires a new ACL
implementation.

### `EmbeddingPort` (ACL — wraps Embedding Provider)

```typescript
interface EmbeddingPort {
  embed(
    texts: string[],
    modelVersion: EmbeddingModelVersion
  ): Promise<number[][]>;
  getDefaultModelVersion(): EmbeddingModelVersion;
}
```

Implementations: OpenAI Ada, local embedding server, or a deterministic mock
for tests. The domain only holds `EmbeddingModelVersion`, never SDK-specific
client objects.

**Why this matters:** PRD Risk 5 — changing the embedding model must trigger
re-indexing. By recording `EmbeddingModelVersion` on every Chunk, the domain
can detect incompatibility at ingestion and query time.

---

## Directory Layout (reference)

```
src/
└── ingestion/
    ├── domain/
    │   ├── IngestionDocument.ts
    │   ├── Chunk.ts
    │   ├── ChunkEmbedding.ts
    │   ├── IngestionRun.ts
    │   ├── ChunkingConfig.ts
    │   ├── events/
    │   │   ├── DocumentIngested.ts
    │   │   ├── ChunksCreated.ts
    │   │   ├── EmbeddingsGenerated.ts
    │   │   └── ChunksIndexed.ts
    │   └── ports/
    │       ├── VectorStorePort.ts
    │       ├── EmbeddingPort.ts
    │       └── FileParserPort.ts
    ├── application/
    │   ├── IngestionOrchestrator.ts
    │   ├── ChunkingService.ts
    │   └── EmbeddingService.ts
    └── infrastructure/
        ├── PostgresVectorStoreAdapter.ts
        ├── OpenAiEmbeddingAdapter.ts
        ├── MockEmbeddingAdapter.ts
        └── parsers/
            ├── MarkdownParser.ts
            └── PlainTextParser.ts
```
