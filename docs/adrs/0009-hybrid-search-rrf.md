# ADR-0009: Hybrid search uses local keyword candidates and application-level RRF

- **Status:** Accepted
- **Date:** 2026-06-09
- **Milestone:** M5 - Hybrid Search
- **Context refs:** PRD §8.5 FR-SRCH-003/004, §8.7 FR-EVAL-002/003, §13 Milestone 5, §15 Risk 2, §16 Open Question 7, `docs/ddd/contexts/retrieval.md`, ADR-0001, ADR-0002, ADR-0003, ADR-0005, `src/retrieval/ports.rs`, `src/retrieval/domain/explain.rs`

## Context
Milestone 5 adds `--mode vector`, `--mode keyword`, and `--mode hybrid`. The product reason is
explicit: embeddings alone often miss exact technical terms, error strings, command names, and package
identifiers, while keyword search alone misses conceptual matches. The PRD also requires mode comparison
through evaluation and says hybrid must improve or equal vector-only Hit@3 on the curated dataset.

The current implementation already constrains the M5 design:

1. Retrieval is a read-only bounded context with its own ports (ADR-0001).
2. Project/tag/source filters are applied in the application layer through `DocumentLookupPort`
   (ADR-0002), so every mode must share the same filter and soft-delete semantics.
3. Vector distances are normalized to `[0,1]` similarity for domain/output scores (ADR-0003).
4. Evaluation already records `SearchMode`, `top_k`, and reports metrics by run (ADR-0005), so M5
   should reuse the same evaluation path instead of creating a special comparator.
5. The embedded `ruvector-core` adapter currently exposes vector k-NN only. Its stored metadata has
   identifiers and previews, not a full lexical index. Keyword mode must not be implemented by matching
   only previews.

The open architecture question is whether hybrid should depend on provider-native hybrid search or be
assembled in tovli's Retrieval application layer.

## Decision
Use **application-level hybrid search** for M5:

- `SearchMode` grows to `Vector`, `Keyword`, and `Hybrid`; `search`, `eval`, and `ask` parse the same
  mode values and pass them through the existing `Query`/`RetrievalRun` contract.
- Keep vector search behind the existing read-only `VectorSearchPort`.
- Add a separate read-only lexical candidate port, `KeywordSearchPort`, returning chunk id, document id,
  source path, heading path, preview, metadata, and a raw lexical score. A matching ingestion-side
  lexical index writer keeps the keyword index in sync with chunk upsert/delete. The default M5 index is
  local and deterministic; provider-native keyword or sparse search can later implement the same port.
- Keyword search indexes full chunk content produced by ingestion. It does not search only vector
  metadata or previews.
- All modes run through the same document lookup, filter, deleted-document exclusion, rank assignment,
  trimming, and explain-output path. This preserves ADR-0002 behavior across modes.

Hybrid mode calls vector and keyword search independently, unions candidates by `chunk_id`, then ranks
with **Reciprocal Rank Fusion (RRF)**:

```text
rrf_raw(chunk) =
  vector_weight  / (RRF_K + vector_rank(chunk))  +
  keyword_weight / (RRF_K + keyword_rank(chunk))

RRF_K = 60
vector_weight = 0.5
keyword_weight = 0.5
```

Ranks are 1-based. A missing rank contributes `0`. The fused score stored in `RetrievalResult.score` is
`rrf_raw / ((vector_weight + keyword_weight) / (RRF_K + 1))`, clamped to `[0,1]`, so domain/output scores
retain the ADR-0003 `[0,1]` contract. `ExplainResultDetail` records the vector score, normalized keyword
score, fused score, and eligibility reason for each emitted result.

Candidate depth for hybrid is intentionally larger than the displayed result count:

```text
hybrid_candidate_k = max(fetch_k(top_k, filters_set), top_k * 5)
```

This gives RRF enough cross-mode evidence while reusing the existing filter over-fetch heuristic.

M5 evaluation runs the same dataset once per mode and writes separate reports. The completion check is:
`Hit@3(hybrid) >= Hit@3(vector)` on the curated dataset, with MRR and empty-result count reviewed as
secondary signals. If hybrid loses to vector-only, the M5 implementation is not done; tune tokenization,
candidate depth, or weights before making hybrid the recommended mode.

## Consequences
- **+** Hybrid ranking is transparent, deterministic, and unit-testable without depending on unstable
  provider-native hybrid APIs.
- **+** Exact identifiers and error strings get a real lexical path, while conceptual matches continue
  to come from vectors.
- **+** Evaluation and RAG do not need a separate code path: they consume `RetrievalRun` with the selected
  `SearchMode`.
- **+** `--explain` can show how each result won: vector-only, keyword-only, or both.
- **-** M5 must maintain a second local index and keep it idempotent with ingestion updates/deletes.
- **-** RRF is rank-based, so it discards raw score magnitude. The explain payload keeps per-mode scores
  visible to make tuning possible.
- **-** Candidate depth and weights are heuristics until eval data proves better values.
- **-** The ADR-0003 weak-retrieval floor is calibrated for vector cosine similarity only. Keyword and
  hybrid scores are mode-relative in M5, so `ask --mode keyword|hybrid` should be reviewed against eval
  data before adding a no-answer floor for those modes.
- **-** The default redb keyword adapter is intentionally lightweight for M5: it stores full chunk content
  plus preview, then scans/deserializes/tokenizes the keyword table per query to compute BM25. This is
  acceptable at milestone scale but should be replaced with a persisted inverted index or cached document
  frequencies when corpus size makes eval latency unacceptable.
- **Future (M9):** RuVector-native sparse or hybrid search can replace the default lexical adapter only
  if it improves quality or latency against the same vector/keyword/hybrid evaluation reports.
