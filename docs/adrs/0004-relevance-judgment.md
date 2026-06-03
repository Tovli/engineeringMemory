# ADR-0004: Relevance judged by exact chunk-id OR path-tolerant source-file match

- **Status:** Accepted
- **Date:** 2026-06-03
- **Milestone:** M3 — Retrieval Evaluation
- **Context refs:** PRD §8.7 FR-EVAL-001/002, `docs/ddd/contexts/evaluation.md`, `src/ingestion/domain.rs` (`chunk_id`), M2 smoke output

## Context
Evaluation must decide whether a returned `RetrievalResult` is "relevant" to an `EvalQuestion`,
which carries optional `expectedChunkIds` and/or `expectedSourceFiles`. Two facts complicate this:

1. **Chunk ids are unstable.** `chunk_id = "chunk_" + blake3(documentId:chunkIndex:contentHash)[..16]`
   (`src/ingestion/domain.rs`). Any edit to a file, a chunk-size config change, or a re-chunk changes
   the id. Hand-authoring `expectedChunkIds` in a dataset is therefore brittle.
2. **Source paths vary in form.** The indexed `source_path` is whatever was passed to `ingest`
   (M2 smoke showed `./docs\deploy.md` on Windows), while a dataset written by a human says
   `docs/deploy.md`. Naive string equality would never match.

## Decision
A result is **relevant** to a question iff **either**:
- `result.chunk_id` is in `expectedChunkIds` (exact match — used when a dataset pins specific chunks), **or**
- `result.source_path` **path-tolerantly** matches any entry in `expectedSourceFiles`, where
  `source_matches(indexed, expected)` normalizes both (backslashes → `/`, strip leading `./`,
  lowercase) and returns true when they are equal, when `indexed` ends with `/expected`, or when
  their basenames are equal.

Source-file matching is the **robust default**; `expectedChunkIds` is the precise-but-optional path.
At load time a question with **neither** field is rejected (evaluation.md EvalQuestion invariant 1).
Only the **first** relevant result (lowest rank) counts, feeding Hit@K and reciprocal rank.

## Consequences
- **+** Datasets stay maintainable: authors reference stable file paths, not volatile chunk hashes.
- **+** Cross-platform: the same dataset evaluates correctly whether ingested on Windows or POSIX.
- **+** `is_relevant` / `source_matches` are pure functions → unit-tested directly (path-form cases, E4).
- **−** Basename matching can over-credit when two different directories hold same-named files
  (e.g. two `README.md`). Acceptable for the curated M3 dataset; a dataset can disambiguate by using
  a longer suffix (`contexts/retrieval.md`), which the `ends_with("/" + expected)` rule honours.
- **−** Source-file granularity treats "any chunk from the right file" as a hit, which is coarser than
  chunk-level relevance. That matches Milestone 3's source-file-oriented dataset and the PRD example;
  chunk-level precision is available by populating `expectedChunkIds`.
