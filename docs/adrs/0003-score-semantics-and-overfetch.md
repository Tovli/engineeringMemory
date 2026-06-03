# ADR-0003: Report cosine similarity (1 − distance) and over-fetch before post-filtering

- **Status:** Accepted
- **Date:** 2026-06-03
- **Milestone:** M2 — Retrieval CLI
- **Context refs:** PRD §11.5, `docs/ddd/contexts/retrieval.md` (RetrievalResult), `ruvector-core` `vector_db.rs::search`, `src/vector_store.rs`

## Context
Two presentation/correctness mismatches between `ruvector-core` and the DDD contract:

1. **Score direction.** `ruvector-core` is configured with `DistanceMetric::Cosine` and returns a
   **distance** (`SearchResult.score`, *lower = closer* — the M0 spike prints "lower = closer").
   The DDD `RetrievalResult.score` is specified as a similarity in **`[0,1]`, higher = better**
   (`retrieval.md`), and Milestone-2 AC-2 expects scores that are highest at rank 1 and
   non-increasing down the list.

2. **Post-filter truncation.** `VectorDB::search` retains filter matches *after* the HNSW top-K
   cut. Because M2 filters in the application layer (ADR-0002), a request for `K` results that also
   applies filters needs **more than `K`** candidates from the store, or filtering can leave too few.

## Decision
- **Normalize score:** the `VectorSearchPort` adapter returns the raw cosine **distance**; the
  application maps it to **`similarity = clamp(1.0 − distance, 0.0, 1.0)`** when building each
  `RetrievalResult`. All domain/display/explain scores are this similarity (higher = better).
  HNSW order is preserved, so similarities are non-increasing by rank → AC-2 holds.
- **Over-fetch:** when any filter is set, fetch `fetch_k = min(K · OVERFETCH, N)` candidates
  (`OVERFETCH = 5`); with no filter, fetch exactly `K`. Filter, then trim to `K`. `N` is the index
  size, so `fetch_k` never exceeds what the store holds.
- **Threshold:** define `SIMILARITY_THRESHOLD = 0.30` (a starting value, tuned in M3). M2 still
  returns below-threshold results but counts them in `RetrievalRun.below_threshold_count` and flags
  them in `--explain`. This wiring is what M4 will use to gate answer generation.

## Consequences
- **+** One consistent score model across CLI output, `--explain`, and the future M3 eval / M4 RAG.
- **+** Over-fetch makes filtered top-K return the intended count whenever enough matches exist.
- **+** `OVERFETCH` and `SIMILARITY_THRESHOLD` are named constants → easy to tune in M3 against the
  evaluation set rather than guesses baked into call sites.
- **−** `OVERFETCH = 5` is heuristic; a query whose filter is very selective could still yield
  `< K` results. Acceptable (AC-3 allows fewer); revisit with eval data in M3.
- **−** Cosine distance in `ruvector-core` can in principle exceed 1.0 (vectors not unit-normalized),
  which would push `1 − distance` negative — hence the `clamp` to `[0,1]`. MiniLM outputs are
  near-unit-norm so this is an edge guard, not the common path.
- **Assumption to verify in Refinement:** that `ruvector-core`'s cosine `distance ≈ 1 − cosine_similarity`.
  A focused unit test (known orthogonal / identical vectors) pins this down before the formatter relies on it.
