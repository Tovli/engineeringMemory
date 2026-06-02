# Shared Kernel — tovli

The Shared Kernel contains value objects and identifiers that are used by
more than one bounded context. Any change here requires coordination across
all contexts that depend on it.

> Source: `docs/prd.md` §11 (data model).

---

## Purpose

Keep the shared surface minimal. Contexts should share only what they cannot
reasonably duplicate or translate. The Shared Kernel is not a "common library"
dumping ground — it is a deliberately small set of stable, well-understood
types.

---

## Shared Identifiers

These are opaque string identifiers. They are defined here once; each context
uses them but never redefines them.

```typescript
// Opaque identifier types (branded strings in implementation)
type DocumentId = string; // prefix: "doc_"
type ChunkId    = string; // prefix: "chk_"
type QueryId    = string; // prefix: "qry_"
type EvalRunId  = string; // prefix: "evr_"
type FeedbackId = string; // prefix: "fbk_"
```

Identifiers are assigned at creation time and never reassigned.

---

## Shared Value Objects

### ContentHash

```typescript
type ContentHash = {
  readonly algorithm: "sha256";
  readonly hex: string; // 64-character lowercase hex string
};
```

Invariants:
- `hex` must be exactly 64 lowercase hex characters.
- `algorithm` is always `"sha256"` in this version.

Used by: Ingestion (Document, Chunk idempotency).

---

### EmbeddingModelVersion

```typescript
type EmbeddingModelVersion = {
  readonly modelName: string; // e.g. "text-embedding-3-small"
  readonly dimension: number; // e.g. 1536
};
```

Invariants:
- `dimension` must be a positive integer.
- Two `EmbeddingModelVersion` values are equal iff both `modelName` and
  `dimension` are equal.
- All Embeddings within a single VectorIndex must share the same
  `EmbeddingModelVersion`. Mixing is prohibited.

Used by: Ingestion (Embedding), Retrieval (Query embedding, VectorIndex
compatibility check), Evaluation (model version in EvalRun report).

---

### SearchMode

```typescript
type SearchMode = "vector" | "keyword" | "hybrid";
```

Used by: Retrieval (RetrievalRun), Evaluation (EvalRun configuration),
Answer Generation (passed through from Query).

---

### MetadataFilter

```typescript
type MetadataFilter = {
  project?: string;
  tag?: string;
  sourcePath?: string;
  fileExtension?: string;
  [key: string]: string | undefined;
};
```

An empty `MetadataFilter` (all fields undefined) means "no filtering".

Used by: Retrieval (applied at query time), Evaluation (per EvalRun), Answer
Generation (forwarded from Query).

---

## What Is NOT in the Shared Kernel

The following types live in their owning context, not here, even though
multiple contexts reference them by identifier:

| Type             | Owning Context | Other Contexts reference by... |
|------------------|----------------|-------------------------------|
| `Document`       | Ingestion      | `DocumentId`                  |
| `Chunk`          | Ingestion      | `ChunkId`                     |
| `RetrievalRun`   | Retrieval      | `SearchExecuted` event payload |
| `Answer`         | RAG            | internal to RAG context        |
| `EvalQuestion`   | Evaluation     | internal to Evaluation context |
| `Feedback`       | Feedback       | `FeedbackId`                  |

This prevents the Shared Kernel from growing into an anemic shared data layer.
