# M6 Feedback and Retrieval Debugging - Architecture

## Modules
```text
src/feedback/
  domain/       FeedbackItem, RetrievalRunEvidence, FeedbackReport value types
  ports.rs      FeedbackRepository, RetrievalRunEvidenceStore
  application/  FeedbackService, FeedbackReportService
  infra/        RedbFeedbackRepository, JsonlRetrievalRunLog, export writer
```

The CLI remains a thin shell over application services. `search` and `ask` create retrieval evidence
snapshots by converting the existing `RetrievalRun` aggregate through `RetrievalRunEvidence::from_run`.

## Storage
- `.tovli/retrieval-runs.jsonl` stores immutable displayed-result snapshots.
- `.tovli/feedback.redb` stores append-only `FeedbackItem` JSON records keyed by feedback id.
- `feedback-report --export <path>` writes the raw feedback items as pretty JSON.

## ADR-0011 Alignment
- Feedback validates against displayed retrieval evidence, not arbitrary chunk ids.
- Reports are derived on demand and are not persisted as authoritative state.
- Re-chunking candidates are advisory only.
- No retrieval ranking code reads feedback data in M6.
