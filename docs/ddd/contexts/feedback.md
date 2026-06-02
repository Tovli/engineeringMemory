# Feedback Bounded Context

> Source: `docs/prd.md` §§8.8, §10, §11.6, §13 (Milestone 6).

---

## Purpose and Responsibility

The Feedback context captures explicit user signals about retrieval quality
and surfaces patterns that guide improvement. It is responsible for:

- Recording a user's good/bad rating for a specific Chunk in the context
  of a specific Query.
- Validating that the referenced QueryId and ChunkId are known.
- Aggregating Feedback into a FeedbackReport that highlights weak retrieval
  patterns: queries with predominantly bad results, frequently downvoted
  Chunks, queries with no useful results.
- Exporting Feedback data for offline analysis.
- Providing the raw signal for future ranking improvements (but NOT
  automatically changing rankings — any learning feature must be toggled
  explicitly and validated by EvalRun comparison, PRD §8.8 FR-FB-003).

The Feedback context does NOT perform retrieval, generate answers, or modify
the vector index.

---

## Aggregates

### `FeedbackItem` (Aggregate Root)

One user rating for one Chunk in the context of one Query.

```typescript
type FeedbackItem = {
  id: FeedbackId;
  queryId: QueryId;
  chunkId: ChunkId;
  rating: FeedbackRating;
  note?: string;           // optional free-text note from user
  sourcePath: string;      // denormalised for reporting, from the RetrievalRun
  questionText: string;    // denormalised for reporting
  createdAt: string;
};

type FeedbackRating = "good" | "bad";
```

**Invariants:**
1. `queryId` must reference an existing Query (validated at creation time).
2. `chunkId` must reference an existing Chunk (validated at creation time).
3. A FeedbackItem is immutable once created. If the user wants to change
   their rating, a new FeedbackItem is created (append-only log).
4. `rating` is exactly `"good"` or `"bad"` — no neutral or numeric scale
   in this version (PRD §8.8 FR-FB-001).

---

### `FeedbackReport` (Aggregate Root)

A derived aggregation computed on demand from the stored FeedbackItems.
Not persisted as a row — generated from queries at report time.

```typescript
type FeedbackReport = {
  id: string;               // "rpt_..."
  generatedAt: string;
  period?: { from: string; to: string }; // optional time window
  problematicQueries: ProblematicQuery[];
  frequentlyDownvotedChunks: DownvotedChunk[];
  queriesWithNoGoodResult: NoGoodResultQuery[];
  candidatesForRechunking: RechunkingCandidate[];
};

type ProblematicQuery = {
  queryId: QueryId;
  questionText: string;
  badCount: number;
  goodCount: number;
  badRatio: number;         // bad / (good + bad)
};

type DownvotedChunk = {
  chunkId: ChunkId;
  sourcePath: string;
  badCount: number;
  distinctQueryCount: number; // how many distinct queries flagged this chunk
};

type NoGoodResultQuery = {
  queryId: QueryId;
  questionText: string;
  totalFeedback: number;
  goodCount: number;
};

type RechunkingCandidate = {
  documentId: DocumentId;
  sourcePath: string;
  downvotedChunkCount: number;
  reason: string; // human-readable suggestion
};
```

**Invariants:**
1. `badRatio` in `ProblematicQuery` is a float in [0.0, 1.0].
2. `RechunkingCandidate` entries are documents where `downvotedChunkCount`
   exceeds a configurable threshold — the domain does not auto-initiate
   re-chunking; it only surfaces the recommendation.

---

## Value Objects

No additional value objects beyond those in the Shared Kernel. FeedbackItem
uses `FeedbackRating` as an enumeration value object.

---

## Domain Events

| Event               | Trigger                                    |
|---------------------|--------------------------------------------|
| `FeedbackRecorded`  | A FeedbackItem is successfully saved       |

Event payloads are in [`../domain-events.md`](../domain-events.md).

---

## Repository Interfaces

```typescript
interface FeedbackRepository {
  save(item: FeedbackItem): Promise<void>;
  findById(id: FeedbackId): Promise<FeedbackItem | null>;
  findByQueryId(queryId: QueryId): Promise<FeedbackItem[]>;
  findByChunkId(chunkId: ChunkId): Promise<FeedbackItem[]>;
  findAll(options?: {
    from?: string;
    to?: string;
    rating?: FeedbackRating;
    limit?: number;
  }): Promise<FeedbackItem[]>;
  exportAll(): Promise<FeedbackItem[]>; // for export feature
}
```

---

## Domain Services

### `FeedbackService` (Domain Service)

```typescript
interface FeedbackService {
  record(
    queryId: QueryId,
    chunkId: ChunkId,
    rating: FeedbackRating,
    note?: string
  ): Promise<FeedbackItem>;
}
```

Internal steps:
1. Validate `queryId` exists (calls `QueryValidationPort`).
2. Validate `chunkId` exists (calls `ChunkValidationPort`).
3. Denormalise `sourcePath` and `questionText` for reporting convenience.
4. Create and persist FeedbackItem.
5. Publish `FeedbackRecorded`.

### `FeedbackReportService` (Domain Service)

```typescript
interface FeedbackReportService {
  generateReport(period?: { from: string; to: string }): Promise<FeedbackReport>;
}
```

Reads all relevant FeedbackItems and computes the derived aggregations.
This is a read-model computation, not a state change — it does not persist the
report (the CLI prints it). The domain service is pure over the data it receives.

---

## Anti-Corruption Layer / Validation Ports

The Feedback context must validate foreign identifiers without importing the
full Ingestion or Retrieval domain models. It uses narrow validation ports:

```typescript
// Feedback's internal port — validates a QueryId exists
interface QueryValidationPort {
  exists(id: QueryId): Promise<boolean>;
  getQuestionText(id: QueryId): Promise<string | null>;
}

// Feedback's internal port — validates a ChunkId exists and gets sourcePath
interface ChunkValidationPort {
  exists(id: ChunkId): Promise<boolean>;
  getSourcePath(id: ChunkId): Promise<string | null>;
}
```

These ports are thin adapters over the shared Postgres database. They do not
import Retrieval or Ingestion domain types — only IDs and the minimal
denormalised data the Feedback context needs for its reports.

---

## Learning / Ranking Experiment Policy

PRD §8.8 FR-FB-003 allows future use of Feedback to improve ranking, but
with strict guards modelled as domain policies:

1. Any Feedback-influenced ranking feature is behind a named experiment flag.
2. An EvalRun must be executed before and after enabling the feature to
   confirm metric improvement (Hit@3, MRR).
3. The flag defaults to off. Turning it on is a deliberate operator action.
4. Feedback data is never deleted to support retrospective analysis.

These policies are enforced by the `FeedbackRankingExperiment` value object
(not fully specified here — deferred to the Advanced RuVector Experiments
milestone, PRD §13 Milestone 9).

---

## Directory Layout (reference)

```
src/
└── feedback/
    ├── domain/
    │   ├── FeedbackItem.ts
    │   ├── FeedbackReport.ts
    │   ├── events/
    │   │   └── FeedbackRecorded.ts
    │   └── ports/
    │       ├── QueryValidationPort.ts
    │       └── ChunkValidationPort.ts
    ├── application/
    │   ├── FeedbackService.ts
    │   └── FeedbackReportService.ts
    └── infrastructure/
        ├── PostgresFeedbackRepository.ts
        ├── PostgresQueryValidationAdapter.ts
        └── PostgresChunkValidationAdapter.ts
```
