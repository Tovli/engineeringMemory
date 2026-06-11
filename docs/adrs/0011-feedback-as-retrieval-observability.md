# ADR-0011: Feedback is append-only retrieval observability, not automatic ranking input

- **Status:** Accepted
- **Date:** 2026-06-10
- **Milestone:** M6 - Feedback and Retrieval Debugging
- **Context refs:** PRD section 8.8 FR-FB-001..003, PRD section 13 Milestone 6, `docs/ddd/contexts/feedback.md`, `docs/ddd/context-map.md`, `docs/ddd/domain-events.md`, ADR-0001, ADR-0002, ADR-0003, ADR-0005, ADR-0008, ADR-0009

## Context
Milestone 6 adds explicit user feedback and retrieval debugging. The PRD requires users to mark retrieved
chunks as useful or not useful, persist that feedback, export it, and generate reports showing bad
retrieval patterns, frequently downvoted chunks, queries with no good result, and candidate documents that
may need re-chunking.

The existing architecture constrains the design:

1. Retrieval owns the search execution contract and emits immutable `RetrievalRun` output (ADR-0001).
2. Result metadata is normalized after document lookup/filtering (ADR-0002), so feedback should refer to
   the final ranked results the user saw, not raw vector or keyword candidates.
3. Scores are mode-specific in M5: vector scores are calibrated cosine similarity, while keyword and
   hybrid scores are rank/fusion outputs (ADR-0003, ADR-0009). Feedback analysis must keep search mode
   visible.
4. Evaluation already provides the quality gate for retrieval changes (ADR-0005). Feedback can identify
   likely problems, but it is not itself proof that a ranking change improved retrieval.
5. RAG depends on cited chunks from a retrieval run (ADR-0008), so answer feedback and retrieval feedback
   should be traceable to the same `query_id`, `retrieval_run_id`, and `chunk_id` evidence.

The open architecture question for M6 is whether captured feedback should immediately influence ranking,
or whether feedback should first be treated as observability data that drives reports and later controlled
experiments.

## Decision
Treat M6 feedback as an append-only observability log tied to retrieval evidence. It does not automatically
change ranking.

- Add a Feedback bounded context with `FeedbackItem` as the persisted aggregate root and `FeedbackReport`
  as a derived, on-demand report.
- Each `FeedbackItem` records one rating for one displayed chunk in the context of one query/run:
  `feedback_id`, `query_id`, `retrieval_run_id`, `chunk_id`, `rating`, optional `note`, `search_mode`,
  `rank`, `score`, `source_path`, `question_text`, and `created_at`.
- Ratings are binary for M6: `good` or `bad`. A correction creates a new item; existing feedback is never
  updated in place or deleted.
- Feedback creation validates that the referenced retrieval run exists and that the chunk was present in
  that run's displayed results. Feedback must not be recorded against arbitrary chunk ids that the user
  did not see.
- The feedback store keeps enough denormalized retrieval evidence for reporting and export even if a
  source document is later re-ingested or soft-deleted.
- `tovli feedback` writes feedback items. `tovli feedback-report` computes reports from stored feedback;
  reports are not persisted as authoritative state.
- Reports group by query/run/chunk/document and include search mode, rank, and score so users can separate
  weak vector retrieval, weak keyword matching, and hybrid-fusion issues.
- Re-chunking output is advisory only. It identifies candidate documents and reasons; it never triggers
  ingestion, deletion, re-chunking, or re-indexing.
- Feedback export emits the append-only records in a stable machine-readable format so external analysis
  can reproduce the report inputs.
- Any future feedback-influenced ranking feature must be behind an explicit experiment flag, default off,
  and must pass before/after eval comparison before becoming a default behavior.

The local M6 persistence default should follow the existing local-first storage style. A redb-backed
feedback repository is acceptable for the CLI milestone, while a later API milestone may add HTTP handlers
over the same application services.

## Consequences
- **+** User ratings remain auditable facts about what was shown, when, and under which search mode.
- **+** Reports can explain bad retrieval without coupling Feedback to vector, keyword, hybrid, or RAG
  implementation details.
- **+** Ranking quality still changes through the existing evaluation gate instead of ad hoc live feedback
  mutation.
- **+** Re-ingestion and soft deletion do not destroy historical feedback context because reporting fields
  are denormalized at capture time.
- **+** The same Feedback services can later be reused by the local API and bot interface.
- **-** M6 must persist retrieval-run evidence, or a feedback-specific snapshot of that evidence, before a
  later command can validate `query_id` / `retrieval_run_id` / `chunk_id`.
- **-** Append-only corrections require reports to handle multiple ratings for the same query/chunk pair
  rather than assuming one mutable final value.
- **-** Feedback does not improve ranking immediately; it only identifies candidates for tuning until a
  separate experiment passes evaluation.
- **Future:** Feedback-driven boosts, penalties, query expansion, reranking, or RuVector self-learning
  features can supersede this analysis-only policy only through a new ADR that defines the experiment flag,
  eval threshold, and rollback behavior.
