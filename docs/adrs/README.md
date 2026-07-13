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
| [0006](0006-llm-provider-abstraction.md) | LLM provider behind an `LlmPort` seam; domain enforces citations | Accepted | M4 |
| [0007](0007-prompt-template-versioning.md) | Versioned prompt templates in source, stamped into every answer log | Accepted | M4 |
| [0008](0008-citation-protocol-and-no-answer-policy.md) | Citation protocol (chunk-id tags) and the no-answer / exit-code policy | Accepted | M4 |
| [0009](0009-hybrid-search-rrf.md) | Hybrid search uses local keyword candidates and application-level RRF | Accepted | M5 |
| [0010](0010-committed-release-versions.md) | Publish crates from committed semantic release versions | Accepted | Release Automation |
| [0011](0011-feedback-as-retrieval-observability.md) | Feedback is append-only retrieval observability, not automatic ranking input | Accepted | M6 |
| [0012](0012-local-api-thin-adapter-over-services.md) | The local API is a thin async adapter over the existing application services | Proposed | M7 |

> ADRs 0001–0003 were produced during the **M2 — Retrieval CLI** SPARC Architecture phase
> (`docs/sparc/m2-retrieval-cli/03-architecture.md`).
> ADRs 0004–0005 during **M3 — Retrieval Evaluation** (`docs/sparc/m3-retrieval-eval/03-architecture.md`).
> ADRs 0006–0008 during **M4 — RAG Answer Generation** (`docs/sparc/m4-rag-answer-generation/03-architecture.md`).
> ADR 0009 captures the **M5 — Hybrid Search** architecture decision from the PRD.
> ADR 0011 captures the **M6 — Feedback and Retrieval Debugging** architecture decision from the PRD.
> ADR 0012 proposes the **M7 — Local API** architecture decision ahead of its SPARC cycle (status `Proposed`).
