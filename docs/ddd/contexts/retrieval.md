# Retrieval Bounded Context

> Source: `docs/prd.md` §§8.5, §10, §11.4–11.5, §15.

---

## Purpose and Responsibility

The Retrieval context is the engine that turns a user's natural-language
question into an ordered list of relevant Chunks. It is responsible for:

- Accepting a Query (question text, SearchMode, MetadataFilter, TopK).
- Embedding the query text using the same EmbeddingModelVersion as the index.
- Executing the selected search strategy: vector, keyword, or hybrid.
- Applying MetadataFilters to narrow the candidate set.
- Producing a ranked RetrievalRun with scored RetrievalResults.
- Enforcing EmbeddingModelVersion compatibility between query and index.
- Providing an ExplainPayload when explain mode is requested.
- Publishing `SearchExecuted` events so downstream contexts (Answer Generation,
  Evaluation, Feedback) can observe the results.

The Retrieval context does NOT generate answers or call the LLM. It does not
write to the vector store (read-only consumer of the index Ingestion populates).

---

## Aggregates

### `RetrievalRun` (Aggregate Root)

The authoritative record of one search execution. Immutable once completed.

```typescript
type RetrievalRun = {
  id: string;                          // "rrun_..."
  query: Query;                        // value object, embedded within
  results: RetrievalResult[];          // ordered by rank ascending (1 = best)
  searchMode: SearchMode;
  topK: number;
  latencyMs: number;
  belowThresholdCount: number;         // results with score < SimilarityThreshold
  explain?: ExplainPayload;            // present only when requested
  completedAt: string;
};
```

**Invariants:**
1. `results` are immutable once the run is completed. A RetrievalRun is
   never modified after creation — only read and published.
2. `results.length` <= `topK`. Fewer results are returned when fewer
   eligible chunks exist.
3. `results` are ordered with rank 1 first (best match).
4. If the `query.embeddingModel` is set and differs from the
   EmbeddingModelVersion of the active index, the run must fail with
   an `EmbeddingModelMismatch` domain error before executing — never
   silently mix dimensions (PRD Risk 5).
5. `latencyMs` is always recorded; it feeds into EvalRun metrics.

---

### Query (Entity, owned by RetrievalRun)

A Query is created when the user issues a search or ask command. It is the
stable identity anchor for a RetrievalRun, an Answer, and Feedback items.

```typescript
type Query = {
  id: QueryId;
  questionText: string;
  searchMode: SearchMode;
  filters: MetadataFilter;
  topK: number;
  embeddingModel?: EmbeddingModelVersion; // null = use active index default
  createdAt: string;
};
```

**Invariants:**
1. `questionText` must be non-empty.
2. `topK` must be a positive integer; default is 8 (PRD §8.5 FR-SRCH-001).
3. Once created, a Query is immutable. Variations on the same question are
   separate Query instances.

---

## Value Objects

### RetrievalResult

```typescript
type RetrievalResult = {
  readonly rank: number;          // 1-based; 1 = most relevant
  readonly chunkId: ChunkId;
  readonly documentId: DocumentId;
  readonly sourcePath: string;
  readonly score: number;         // similarity/relevance score
  readonly preview: string;
  readonly headingPath: string[];
  readonly metadata: Record<string, string | number | boolean>;
};
```

Invariants:
- `rank` >= 1 and unique within a RetrievalRun.
- `score` is a float in [0.0, 1.0] for cosine similarity; BM25 scores are
  normalised to the same range before fusion in hybrid mode.

---

### ExplainPayload

```typescript
type ExplainPayload = {
  readonly queryEmbeddingProvider: string;  // model name
  readonly queryEmbeddingDimension: number;
  readonly searchMode: SearchMode;
  readonly filtersApplied: MetadataFilter;
  readonly rankingMethod: string;           // e.g. "cosine" or "rrf"
  readonly resultDetails: ExplainResultDetail[];
};

type ExplainResultDetail = {
  readonly chunkId: ChunkId;
  readonly rank: number;
  readonly vectorScore?: number;
  readonly keywordScore?: number;
  readonly fusedScore: number;
  readonly eligibilityReason: string;
};
```

The ExplainPayload is the observability artefact that makes retrieval
debuggable (PRD §7 "Observability Is a Product Feature", §8.5 FR-SRCH-004).

---

## Domain Events

| Event              | Trigger                                       |
|--------------------|-----------------------------------------------|
| `SearchExecuted`   | A RetrievalRun is completed successfully      |
| `SearchFailed`     | A RetrievalRun fails (e.g. model mismatch)    |

Event payloads are in [`../domain-events.md`](../domain-events.md).

---

## Repository Interfaces

```typescript
interface RetrievalRunRepository {
  save(run: RetrievalRun): Promise<void>;
  findById(id: string): Promise<RetrievalRun | null>;
  findByQueryId(queryId: QueryId): Promise<RetrievalRun[]>;
}

interface QueryRepository {
  save(query: Query): Promise<void>;
  findById(id: QueryId): Promise<Query | null>;
}
```

---

## Domain Services

### `SearchService` (Domain Service, depends on VectorStorePort + EmbeddingPort ACLs)

Responsibility: execute a retrieval strategy for a given Query.

```typescript
interface SearchService {
  search(query: Query): Promise<RetrievalRun>;
}
```

Internal steps:
1. Validate EmbeddingModelVersion compatibility with the active index.
2. Embed `query.questionText` via `EmbeddingPort`.
3. Dispatch to the appropriate strategy:
   - `VectorSearchStrategy`: calls `VectorStorePort.vectorSearch(...)`.
   - `KeywordSearchStrategy`: calls `VectorStorePort.keywordSearch(...)`.
   - `HybridSearchStrategy`: calls both and fuses scores (RRF or
     provider-native fusion).
4. Apply MetadataFilters.
5. Rank and trim to TopK.
6. Record latency, build RetrievalRun, publish `SearchExecuted`.

The search strategy implementations are behind a `SearchStrategy` interface
so new modes (e.g. sparse vectors) can be added without changing the domain.

### `ScoreFusionService` (Domain Service, used by HybridSearchStrategy)

Responsibility: fuse vector scores and keyword scores into a single ranked list.
Uses Reciprocal Rank Fusion (RRF) by default. The fusion method is recorded
in `ExplainPayload.rankingMethod` for observability.

---

## Anti-Corruption Layer

### `VectorStorePort` (ACL — wraps RuVector/Postgres, read side)

```typescript
interface VectorStorePort {
  vectorSearch(
    queryVector: number[],
    modelVersion: EmbeddingModelVersion,
    filters: MetadataFilter,
    topK: number
  ): Promise<RawSearchResult[]>;

  keywordSearch(
    queryText: string,
    filters: MetadataFilter,
    topK: number
  ): Promise<RawSearchResult[]>;

  getIndexedModelVersion(): Promise<EmbeddingModelVersion | null>;
}

// ACL-internal type — never exposed to the domain
type RawSearchResult = {
  chunkId: string;
  documentId: string;
  sourcePath: string;
  score: number;
  preview: string;
  headingPath: string[];
  metadata: Record<string, unknown>;
};
```

The domain receives `RawSearchResult[]` from the ACL and maps it to
`RetrievalResult[]`. This translation layer isolates the domain from
RuVector-specific score formats and SQL result shapes.

### `EmbeddingPort` (ACL — wraps Embedding Provider, query side)

```typescript
interface EmbeddingPort {
  embed(text: string, modelVersion: EmbeddingModelVersion): Promise<number[]>;
}
```

Query-time embedding is a single-text operation (vs. batch for ingestion).
The same ACL interface is shared with the Ingestion context (they may share
the same adapter or use separate instances).

---

## Key Design Decisions

### Why Retrieval is Read-Only
The Retrieval context only reads the vector index. It never writes to it.
This respects the upstream-supplier relationship with Ingestion and prevents
two contexts from owning the same data.

### Embedding Model Mismatch Detection
Before executing a search, `SearchService` calls
`VectorStorePort.getIndexedModelVersion()` and compares it with the query's
`embeddingModel`. If they differ, a `SearchFailed` event is published with
reason `"EmbeddingModelMismatch"` and the user is instructed to run
`tovli reembed` (PRD §8.3 FR-EMB-002).

### Observability
Every `SearchExecuted` event includes `latencyMs`, `searchMode`,
`filtersApplied`, and result scores. The Evaluation context relies on
`latencyMs` for its "average retrieval latency" metric.

---

## Directory Layout (reference)

```
src/
└── retrieval/
    ├── domain/
    │   ├── RetrievalRun.ts
    │   ├── Query.ts
    │   ├── RetrievalResult.ts
    │   ├── ExplainPayload.ts
    │   ├── SearchStrategy.ts
    │   ├── events/
    │   │   ├── SearchExecuted.ts
    │   │   └── SearchFailed.ts
    │   └── ports/
    │       ├── VectorStorePort.ts
    │       └── EmbeddingPort.ts
    ├── application/
    │   ├── SearchService.ts
    │   ├── ScoreFusionService.ts
    │   └── strategies/
    │       ├── VectorSearchStrategy.ts
    │       ├── KeywordSearchStrategy.ts
    │       └── HybridSearchStrategy.ts
    └── infrastructure/
        ├── PostgresVectorStoreAdapter.ts
        └── OpenAiEmbeddingAdapter.ts
```
