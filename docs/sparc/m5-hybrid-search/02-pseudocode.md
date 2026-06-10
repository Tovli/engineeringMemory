# SPARC Phase 2 - Pseudocode: M5 Hybrid Search

Notation: `K` = requested top-k, `F` = existing filter over-fetch factor, `C` = candidate depth.

## Ingestion keyword index sync
```text
FN ingest_file(file):
    chunks = chunk(parsed document)
    vectors = embed(chunks)

    IF existing document:
        vector_store.delete_by_document(document_id)
        keyword_index.delete_by_document(document_id)

    vector_store.upsert_chunks(chunks + vectors)
    keyword_index.upsert_chunks(chunks with full content)
    document_repo.save(document)
```

Deletion pass:
```text
FOR active document under root not seen:
    document_repo.soft_delete(id)
    vector_store.delete_by_document(id)
    keyword_index.delete_by_document(id)
```

## Search mode dispatch
```text
FN search(query, explain):
    validate non-empty query and top_k > 0
    indexed_model = lookup.indexed_model_version()
    IF indexed_model is None:
        RETURN empty_run(IndexEmpty)

    uses_vector = query.mode IN {Vector, Hybrid}
    uses_keyword = query.mode IN {Keyword, Hybrid}

    IF uses_vector AND query.embedding_model incompatible with indexed_model:
        ERROR EmbeddingModelMismatch

    filters_set = !query.filters.is_empty()
    single_mode_k = fetch_k(K, filters_set)
    hybrid_k = max(single_mode_k, K * 5)

    vector_candidates = []
    IF uses_vector:
        qvec = embedder.embed_batch([query.text])[0]
        raw_vector = vector_port.vector_search(qvec, uses_keyword ? hybrid_k : single_mode_k)
        vector_candidates = enumerate ranks and normalize cosine distance to similarity

    keyword_candidates = []
    IF uses_keyword:
        raw_keyword = keyword_port.keyword_search(query.text, uses_vector ? hybrid_k : single_mode_k)
        keyword_candidates = enumerate ranks and normalize lexical scores to [0,1]

    candidates =
        IF Vector: vector candidates, score = vector_score
        IF Keyword: keyword candidates, score = keyword_score
        IF Hybrid: union by chunk_id, score = normalized_rrf(vector_rank, keyword_rank)

    docs = lookup.find_many(distinct candidate.document_id)
    kept = []
    FOR candidate sorted by mode score:
        doc = docs[candidate.document_id]
        IF passes filters and doc active:
            kept.push(candidate)
        IF kept.len == K:
            BREAK

    results = kept mapped to RetrievalResult rank 1..K
    explain = optional ExplainPayload with ranking method cosine|keyword|rrf
    RETURN RetrievalRun
```

## RRF scoring
```text
rrf_raw =
    vector_rank  ? 0.5 / (60 + vector_rank)  : 0
  + keyword_rank ? 0.5 / (60 + keyword_rank) : 0

max_raw = (0.5 + 0.5) / (60 + 1)
fused = clamp(rrf_raw / max_raw, 0, 1)
```

## Keyword scoring
```text
FN keyword_search(query, k):
    query_terms = tokenize(query)
    IF query_terms empty: RETURN []

    chunks = scan keyword index
    doc_freqs = count how many chunks contain each query term
    avgdl = average chunk term count

    FOR chunk IN chunks:
        score = BM25(query_terms, chunk_terms, doc_freqs, avgdl)
        IF score > 0:
            scored.push(chunk, score)

    sort by score desc, then source path, then chunk id
    RETURN scored.take(k)
```

## Phase-2 Gate
- [x] Pseudocode covers ingestion sync, mode dispatch, RRF, keyword scoring, and filtering.
- [x] Error paths for empty index, model mismatch, and empty lexical matches are explicit.

**Gate result: PASS.** -> Phase 3 (Architecture).
