# SPARC Phases 4-5 - Refinement & Completion: M5 Hybrid Search

Status: **complete** - `vector`, `keyword`, and `hybrid` modes work through `search`, `eval`, and `ask`;
keyword indexing is synchronized by ingestion; hybrid ranking uses application-level RRF.

## What was built
```text
src/lexical_index.rs                         # keyword record shape, tokenizer, BM25 scoring
src/ingestion/ports.rs                       # + KeywordIndexPort
src/ingestion/orchestrator.rs                # sync keyword index on upsert/reindex/delete
src/ingestion/infra/redb_keyword_index.rs    # redb keyword-index writer
src/retrieval/ports.rs                       # + KeywordSearchPort, RawKeywordSearchResult
src/retrieval/application/scoring.rs         # hybrid_candidate_k + normalized RRF
src/retrieval/application/search_service.rs  # vector/keyword/hybrid dispatch and explain
src/retrieval/infra/redb_keyword_search.rs   # redb keyword search reader
src/main.rs                                  # mode parsing and adapter wiring for search/eval/ask
tests/retrieval_search.rs                    # real keyword-index keyword/hybrid integration
tests/evaluation_eval.rs                     # keyword-mode eval integration
```

## Test results
- `cargo test --no-default-features` -> **71 unit tests + 1 ask IT + 1 default-feature IT + 3 eval ITs + 4 retrieval ITs passed**.
- `cargo test` -> same suite with default ONNX feature enabled, **all passed**.
- `cargo clippy --all-targets --all-features` -> **clean**.

## Traceability matrix
| AC | Covered by |
|----|------------|
| AC-1 keyword exact terms | IT `keyword_and_hybrid_search_use_the_real_keyword_index`; UT `keyword_mode_ranks_by_normalized_keyword_score_without_vector_scores` |
| AC-2 hybrid explain `rrf` | IT `keyword_and_hybrid_search_use_the_real_keyword_index`; UT `hybrid_mode_fuses_vector_and_keyword_ranks_with_rrf` |
| AC-3 RRF boosts dual hits | UT `rrf_score_rewards_chunks_seen_by_both_modes`; UT `hybrid_mode_fuses_vector_and_keyword_ranks_with_rrf` |
| AC-4 shared filters | Existing retrieval filter tests + SearchService shared candidate filter path |
| AC-5 stale keyword deletion | UT `syncs_keyword_index_on_insert_modify_and_delete` |
| AC-6 eval by mode | IT `end_to_end_eval_runs_keyword_mode`; existing report writer path |
| AC-7 invalid mode | UT `search_mode_rejects_invalid_value_with_accepted_values`; CLI exits 2 via `parse_mode_or_exit` |

## Refinement notes
- `KeywordIndexPort` is optional on `IngestionOrchestrator` so older tests and vector-only setups can run
  without a keyword index, while the CLI always wires `RedbKeywordIndex`.
- Keyword mode skips the embedding-model mismatch guard because it does not embed the query. Vector and
  hybrid still enforce model compatibility before vector search.
- The local keyword adapter uses BM25-style scoring over full chunk content and normalizes scores in the
  application layer before exposing them as domain scores. These are mode-relative relevance scores, not
  calibrated cosine similarities; the ADR-0003 similarity threshold is applied only to vector-mode scores.
  Consequently, keyword/hybrid `ask` no-answer behavior needs a future calibrated floor if product wants
  weak-retrieval refusal outside vector mode.
- The redb keyword reader currently scans the persisted keyword chunk table, deserializes matching records,
  and computes BM25/DF values at query time. This keeps M5 local and dependency-light; a persisted inverted
  index or DF cache is the next scaling optimization if eval latency grows with corpus size. The keyword
  table also stores both full content and preview for simple retrieval output, duplicating some text.
- Hybrid mode uses rank-only RRF as ADR-0009 specifies; raw vector/keyword scores remain visible through
  `--explain`.

## Phase-4 Gate
- [x] Tests were written before the production code for the new mode parser, RRF helpers, ingestion sync,
      keyword ranking, and hybrid fusion behavior.
- [x] Hexagonal boundaries hold: keyword writes live in Ingestion; keyword reads live behind Retrieval's
      `KeywordSearchPort`; domain types do not import `redb`.
- [x] Existing vector search, eval, and ask behavior stays covered by the old tests.

## Phase-5 Gate
- [x] All verification commands passed.
- [x] README updated for current modes.
- [x] ADR-0009 implemented without changing downstream `RetrievalRun` consumers.

**SPARC workflow for M5 - Hybrid Search: COMPLETE.**
