# SPARC Phase 3 - Architecture: M5 Hybrid Search

M5 extends the existing Retrieval bounded context without changing downstream contracts. Evaluation and
Answer Generation keep consuming `RetrievalRun`; only the selected `SearchMode` changes.

## Module layout
```text
src/
  lexical_index.rs                         # local keyword index record shape, tokenizer, BM25 helpers
  ingestion/
    ports.rs                               # + KeywordIndexPort
    orchestrator.rs                        # + keyword index sync on upsert/delete
    infra/redb_keyword_index.rs            # writes full chunk content to redb keyword index
  retrieval/
    domain/query.rs                        # SearchMode = Vector | Keyword | Hybrid, FromStr
    ports.rs                               # + KeywordSearchPort, RawKeywordSearchResult
    application/scoring.rs                 # + hybrid_candidate_k, rrf_score
    application/search_service.rs          # mode dispatch + RRF assembly
    infra/redb_keyword_search.rs           # reads keyword index and scores BM25-style candidates
  main.rs                                  # parse modes for search/eval/ask, wire keyword adapter
docs/sparc/m5-hybrid-search/
  01-specification.md
  02-pseudocode.md
  03-architecture.md
  04-refinement-completion.md
```

## Dependencies
```text
Ingestion application -> ingestion::ports::KeywordIndexPort
RedbKeywordIndex      -> lexical_index + ingestion::ports

Retrieval application -> retrieval::ports::{VectorSearchPort, KeywordSearchPort, DocumentLookupPort}
RedbKeywordSearch     -> lexical_index + retrieval::ports

CLI wires both concrete adapters; domain stays pure.
```

`lexical_index.rs` is shared infrastructure support, not domain logic. It contains serialization and
tokenization helpers used by the write adapter and read adapter.

## Contracts
```rust
pub trait KeywordIndexPort {
    fn upsert_chunks(&self, chunks: &[&Chunk]) -> anyhow::Result<()>;
    fn delete_by_document(&self, id: &DocumentId) -> anyhow::Result<()>;
}

pub trait KeywordSearchPort {
    fn keyword_search(&self, query: &str, k: usize) -> anyhow::Result<Vec<RawKeywordSearchResult>>;
}
```

`SearchService` receives both read ports. Vector mode ignores the keyword port; keyword mode ignores the
vector port and query embedder after validation; hybrid uses both.

## Decisions Applied
- ADR-0009 RRF constants live in `retrieval::application::scoring`.
- Keyword index stores full chunk content so exact technical terms are searchable.
- Keyword score is normalized by the highest lexical score in the candidate set before becoming a
  domain score.
- Hybrid score is normalized RRF and remains in `[0,1]`, preserving ADR-0003.
- Embedding-model mismatch remains fatal for vector/hybrid. Keyword mode can run without a vector
  embedding match because it does not embed the query.

## Phase-3 Gate
- [x] File ownership is explicit.
- [x] Port contracts keep domain/application testable with fakes.
- [x] No circular dependency between Ingestion and Retrieval.

**Gate result: PASS.** -> Phase 4 (Refinement).
