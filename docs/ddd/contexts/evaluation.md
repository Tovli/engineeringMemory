# Evaluation Bounded Context

> Source: `docs/prd.md` §§8.7, §10, §13 (Milestone 3), §15.

---

## Purpose and Responsibility

The Evaluation context makes retrieval quality measurable. It is responsible
for:

- Managing the evaluation dataset (EvalQuestions) that defines ground truth.
- Executing an EvalRun: sending each EvalQuestion through the Retrieval context
  and comparing the returned RetrievalRun against expected chunks/sources.
- Computing Hit@1, Hit@3, Hit@5, MRR, average latency, empty-result count,
  and below-threshold count.
- Saving EvalRun reports as structured JSON artefacts.
- Supporting CI-compatible failure mode (`--fail-below-hit-at-3`).
- Enabling comparison across SearchModes (vector vs keyword vs hybrid).

The Evaluation context does NOT modify the vector index, change embeddings,
or generate answers. It only reads the output of the Retrieval context.

The Evaluation context embodies the "Retrieval Before Generation" principle:
it is designed to be built and used before the RAG layer exists (Milestone 3
precedes Milestone 4 in the PRD).

---

## Aggregates

### `EvalRun` (Aggregate Root)

One execution of the full evaluation dataset against a specific search
configuration.

```typescript
type EvalRun = {
  id: EvalRunId;
  datasetPath: string;           // path to the questions JSON file
  searchMode: SearchMode;
  topK: number;
  embeddingModelVersion?: EmbeddingModelVersion;
  status: EvalRunStatus;
  metrics: EvalMetrics;          // computed after all questions are run
  questionResults: EvalQuestionResult[];
  thresholdConfig?: ThresholdConfig;
  startedAt: string;
  completedAt?: string;
  reportPath?: string;           // path where JSON report was written
};

type EvalRunStatus = "running" | "completed" | "failed" | "threshold_failed";
```

**Invariants:**
1. `questionResults.length` equals the number of EvalQuestions in the dataset
   when the run completes. Partial runs that abort mid-way have `status: "failed"`.
2. `metrics` is only populated when `status` is `"completed"` or
   `"threshold_failed"`.
3. If `thresholdConfig` is set and the computed `hitAt3` is below
   `thresholdConfig.minHitAt3`, the EvalRun completes with
   `status: "threshold_failed"` and the CLI exits with a non-zero code.
   This enables CI regression testing (PRD §8.7 FR-EVAL-003).

---

### EvalQuestion (Entity, owned by Evaluation context)

A single test case in the ground-truth dataset.

```typescript
type EvalQuestion = {
  id: string;                    // e.g. "q_001"
  questionText: string;
  expectedChunkIds?: ChunkId[];
  expectedSourceFiles?: string[];
};
```

**Invariants:**
1. At least one of `expectedChunkIds` or `expectedSourceFiles` must be
   provided (otherwise there is no ground truth to evaluate against).
2. EvalQuestions are immutable static artefacts — the system never modifies
   them. They are read from a JSON file, not stored in the primary database.

---

## Value Objects

### EvalMetrics

```typescript
type EvalMetrics = {
  readonly hitAt1: number;           // fraction [0.0, 1.0]
  readonly hitAt3: number;
  readonly hitAt5: number;
  readonly mrr: number;              // Mean Reciprocal Rank
  readonly avgLatencyMs: number;
  readonly emptyResultCount: number; // questions with zero results returned
  readonly belowThresholdCount: number; // questions with no result above threshold
  readonly questionCount: number;
};
```

Invariants:
- All rate values (`hitAt1`, `hitAt3`, `hitAt5`, `mrr`) are in [0.0, 1.0].
- `emptyResultCount` <= `questionCount`.

---

### EvalQuestionResult

```typescript
type EvalQuestionResult = {
  readonly questionId: string;
  readonly questionText: string;
  readonly retrievalRunId: string;
  readonly returnedChunkIds: ChunkId[];
  readonly returnedSourcePaths: string[];
  readonly hitAt1: boolean;
  readonly hitAt3: boolean;
  readonly hitAt5: boolean;
  readonly reciprocalRank: number;   // 0 if not found
  readonly latencyMs: number;
  readonly topScore?: number;
};
```

One `EvalQuestionResult` per EvalQuestion. Owned by the EvalRun aggregate.

---

### ThresholdConfig

```typescript
type ThresholdConfig = {
  readonly minHitAt3?: number;   // e.g. 0.8 → fail if hitAt3 < 0.8
  readonly minHitAt1?: number;
  readonly minMrr?: number;
};
```

---

## Domain Events

| Event                  | Trigger                                          |
|------------------------|--------------------------------------------------|
| `EvaluationCompleted`  | An EvalRun finishes, whether pass or threshold_failed |

Event payloads are in [`../domain-events.md`](../domain-events.md).

---

## Repository Interfaces

```typescript
interface EvalRunRepository {
  save(run: EvalRun): Promise<void>;
  findById(id: EvalRunId): Promise<EvalRun | null>;
  findAll(limit: number): Promise<EvalRun[]>;  // for comparison reports
  findBySearchMode(mode: SearchMode): Promise<EvalRun[]>;
}
```

EvalQuestions are not stored in the database — they are loaded from a JSON
file at runtime. The `EvalRunRepository` only persists completed EvalRuns.

---

## Domain Services

### `EvaluationService` (Domain Service)

```typescript
interface EvaluationService {
  runEvaluation(
    questions: EvalQuestion[],
    config: EvalRunConfig
  ): Promise<EvalRun>;
}

type EvalRunConfig = {
  searchMode: SearchMode;
  topK: number;
  thresholdConfig?: ThresholdConfig;
  embeddingModelVersion?: EmbeddingModelVersion;
  outputPath?: string;
};
```

Internal steps:
1. Create an EvalRun aggregate with `status: "running"`.
2. For each EvalQuestion:
   a. Construct a Query with the question text and evaluation config.
   b. Call the Retrieval context's SearchService (via an injected port).
   c. Receive the RetrievalRun.
   d. Compute HitAtK and reciprocalRank by matching returned chunk IDs
      and source paths against `expectedChunkIds` / `expectedSourceFiles`.
   e. Record EvalQuestionResult.
3. Compute aggregate EvalMetrics.
4. Check ThresholdConfig — set status to `"threshold_failed"` if violated.
5. Persist the EvalRun and optionally write the JSON report.
6. Publish `EvaluationCompleted`.

### `MetricsCalculationService` (Domain Service)

Responsibility: compute EvalMetrics from a list of EvalQuestionResults.
Pure function — no I/O, fully testable without infrastructure.

```typescript
function computeMetrics(results: EvalQuestionResult[]): EvalMetrics;
```

---

## Relationship with Retrieval Context (Conformist)

The Evaluation context conforms to the Retrieval context's output model.
It calls Retrieval's SearchService through a `SearchPort` interface to
preserve loose coupling:

```typescript
// Evaluation's internal port — wraps Retrieval's SearchService
interface SearchPort {
  search(query: Query): Promise<RetrievalRun>;
}
```

Evaluation never imports Retrieval's infrastructure types. If Retrieval
changes its internal model, Evaluation's conformist adapter must update,
but the Evaluation domain objects remain stable.

---

## Observability

EvalRun reports are saved as structured JSON (PRD §8.7 FR-EVAL-002):

```typescript
type EvalReport = {
  runId: EvalRunId;
  generatedAt: string;
  searchMode: SearchMode;
  topK: number;
  embeddingModelVersion?: EmbeddingModelVersion;
  metrics: EvalMetrics;
  questionResults: EvalQuestionResult[];
};
```

Reports can be diffed between runs to detect regressions. This is the
mechanism for "compare multiple search modes" (FR-EVAL-002) and prompt
regression testing (FR-RAG-004).

---

## Directory Layout (reference)

```
src/
└── evaluation/
    ├── domain/
    │   ├── EvalRun.ts
    │   ├── EvalQuestion.ts
    │   ├── EvalMetrics.ts
    │   ├── EvalQuestionResult.ts
    │   ├── ThresholdConfig.ts
    │   ├── events/
    │   │   └── EvaluationCompleted.ts
    │   └── ports/
    │       └── SearchPort.ts
    ├── application/
    │   ├── EvaluationService.ts
    │   ├── MetricsCalculationService.ts
    │   └── EvalReportWriter.ts
    └── infrastructure/
        └── RetrievalSearchAdapter.ts
```
