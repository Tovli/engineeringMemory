# M6 Feedback and Retrieval Debugging - Specification

## Scope
Implement [ADR-0011](../../adrs/0011-feedback-as-retrieval-observability.md): feedback is an
append-only observability signal tied to retrieval evidence, not automatic ranking input.

## Requirements
- `search` and `ask` print stable `query-id` and `run-id` values and persist the displayed retrieval
  results as evidence for later feedback validation.
- `tovli feedback` records `good` / `bad` ratings for chunks that were displayed in a prior retrieval run.
- Feedback records are append-only; changing a rating creates another item.
- `tovli feedback-report` computes problematic queries, downvoted chunks, no-good-result queries, and
  re-chunking candidates.
- Raw feedback can be exported as machine-readable JSON.
- Ranking is not changed by M6 feedback.

## Acceptance Criteria
- Feedback rejects a chunk id that was not displayed in the referenced retrieval run.
- Feedback stores query/run/chunk ids, rating, note, search mode, rank, score, source path, question text,
  and timestamp.
- Reports include the M6 aggregation categories from the PRD.
- Existing retrieval, RAG, and eval tests continue to pass.
