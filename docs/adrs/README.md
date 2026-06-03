# Architecture Decision Records

Lightweight ADRs for tovli. One file per decision, numbered sequentially. Format: MADR-style
(Context · Decision · Consequences). Status: `Proposed` → `Accepted` → (`Superseded by NNNN`).

| ADR | Title | Status | Milestone |
|-----|-------|--------|-----------|
| [0001](0001-retrieval-bounded-context.md) | Retrieval as its own read-only hexagonal module | Accepted | M2 |
| [0002](0002-project-tag-filter-join.md) | Apply project/tag/source filters via a document-lookup join, not the vector store | Accepted | M2 |
| [0003](0003-score-semantics-and-overfetch.md) | Report cosine similarity (1 − distance) and over-fetch before post-filtering | Accepted | M2 |
| [0004](0004-relevance-judgment.md) | Relevance via exact chunk-id OR path-tolerant source-file match | Accepted | M3 |
| [0005](0005-eval-depth-and-ci-determinism.md) | Fixed eval retrieval depth (≥5) and deterministic CI without ONNX | Accepted | M3 |

> ADRs 0001–0003 were produced during the **M2 — Retrieval CLI** SPARC Architecture phase
> (`docs/sparc/m2-retrieval-cli/03-architecture.md`).
> ADRs 0004–0005 during **M3 — Retrieval Evaluation** (`docs/sparc/m3-retrieval-eval/03-architecture.md`).
