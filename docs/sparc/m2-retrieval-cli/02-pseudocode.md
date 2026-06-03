# SPARC Phase 2 — Pseudocode: M2 Retrieval CLI

Algorithms for the **Retrieval** context. Language-neutral, annotated with complexity and the
acceptance criteria / edge cases each step covers. Implements the Phase-1 spec; concrete Rust
types land in Phase 3 (Architecture).

Notation: `N` = chunks in the index, `K` = requested `topK`, `F` = over-fetch factor,
`C = K·F` = candidates pulled from the vector store, `D` = distinct documents among candidates.

---

## Top-level: `search` command

```
FN cli_search(args):                                  # main.rs — thin (R9, C5)
    query_text   = args.positional                    # the question
    IF blank(query_text): ERROR "query must not be empty"; EXIT 2     # E1
    IF args.top_k <= 0:   ERROR "top-k must be a positive integer"; EXIT 2   # E2
    IF args.mode != "vector":                          # M2 scope
        ERROR "mode '{mode}' is available in Milestone 5; use --mode vector"; EXIT 2

    embedder = build_embedder(args.mock)               # same builder as ingest (C2)
    store    = open_vector_search_port(read_only)      # ruvector-core VectorDB  (C3,C6)
    lookup   = open_document_lookup_port(read_only)    # documents.redb          (C3,C6)

    query = Query{ text: query_text, mode: Vector,
                   filters: MetadataFilter{ project, tags, source },
                   top_k: args.top_k,
                   embedding_model: embedder.model_version() }     # R7

    run = SearchService{embedder, store, lookup}.search(query, explain=args.explain)
    print_results(run, query)                          # formatter (R2, AC-1/2)
    EXIT 0
```

---

## `SearchService.search(query, explain) -> RetrievalRun`   (application core)

```
FN search(query, explain):
    t0 = now()

    # 1. Guard rails -------------------------------------------------------- (R6, AC-7/8, E3/E8)
    validate(query)                                    # non-empty text, top_k > 0  (E1,E2)
    indexed = lookup.indexed_model_version()           # from documents.redb; None if index empty
    IF indexed IS None:
        RETURN empty_run(query, reason=INDEX_EMPTY)    # AC-8 — caller prints + exit 0  (E3)
    IF indexed != query.embedding_model:               # name AND dimension must match
        RAISE EmbeddingModelMismatch(indexed, query.embedding_model)   # AC-7, E8 — no search

    # 2. Embed the query ---------------------------------------------------- (R1, C4)
    qvec = embedder.embed_batch([query.text])[0]       # single text; len == dimension
    ASSERT len(qvec) == indexed.dimension              # defense-in-depth vs E8

    # 3. Over-fetch candidates --------------------------------------------- (E5, ADR-0003)
    #    Post-filtering happens in-app (step 5), so pull more than K when filters are set.
    fetch_k = (query.filters.is_empty() ? K : min(K * OVERFETCH, N))   # OVERFETCH = 5, cap K*F<=N
    raw = store.vector_search(qvec, fetch_k)           # ruvector-core knn; NO native filter
    #    raw: [{chunk_id, document_id, source_path, distance, preview, heading_path, metadata}]
    #    cost: HNSW search ~ O(fetch_k · log N)         # dominates only for large N; embed dominates ONNX

    # 4. Resolve document metadata for the join ---------------------------- (E6, E9, ADR-0002)
    doc_ids = distinct(r.document_id for r in raw)     # D <= C
    docs    = lookup.find_many(doc_ids)                # batch read documents.redb; O(D)
    #    docs[id] -> { project, tags[], source_path, status }

    # 5. Apply filters in the application layer ---------------------------- (R3, AC-4, E4/E6/E9)
    kept = []
    FOR r IN raw:                                       # preserves knn order
        doc = docs.get(r.document_id)
        IF doc IS None OR doc.status == Deleted: CONTINUE          # E9 — drop orphan/deleted
        IF query.filters.project AND doc.project != filters.project: CONTINUE
        IF query.filters.tags    AND NOT all_in(filters.tags, doc.tags): CONTINUE   # multi-valued AND
        IF query.filters.source  AND r.source_path != filters.source: CONTINUE
        kept.append((r, doc))

    # 6. Normalize score, rank, trim --------------------------------------- (E7, AC-2/3, ADR-0003)
    results = []
    FOR (i, (r, doc)) IN enumerate(kept[:K]):           # already knn-ordered; trim to K  (AC-3)
        sim = clamp(1.0 - r.distance, 0.0, 1.0)         # cosine distance -> similarity  (E7)
        results.append(RetrievalResult{
            rank: i + 1,                                # 1-based, unique, ascending  (AC-2)
            chunk_id: r.chunk_id, document_id: r.document_id,
            source_path: r.source_path, score: sim,
            preview: r.preview, heading_path: r.heading_path,
            metadata: r.metadata })
    #    `kept` is in non-increasing similarity order because distance is non-decreasing in knn order.

    below = count(res for res in results if res.score < SIMILARITY_THRESHOLD)   # E10

    # 7. Optional explain payload ------------------------------------------ (R5, AC-6)
    explain_payload = explain ? build_explain(query, indexed, results, raw) : None

    # 8. Assemble immutable run + emit event ------------------------------- (R8, observability §9.3)
    run = RetrievalRun{ id: rrun_id(), query, results, search_mode: Vector,
                        top_k: K, latency_ms: now() - t0,
                        below_threshold_count: below, explain: explain_payload,
                        completed_at: now_iso() }
    log_json("SearchExecuted", { run.id, mode, K, latency_ms, n_results: len(results),
                                 filters: query.filters, scores: [r.score for r in results] })
    RETURN run                                          # immutable thereafter (R8 invariant)
```

**On `EmbeddingModelMismatch`** the caller logs `SearchFailed{reason:"EmbeddingModelMismatch"}`
and prints the actionable message (AC-7).

---

## `build_explain(query, indexed, results, raw) -> ExplainPayload`   (R5, AC-6)

```
FN build_explain(query, indexed, results, raw):
    RETURN ExplainPayload{
        query_embedding_provider: indexed.name,
        query_embedding_dimension: indexed.dimension,
        search_mode: Vector,
        filters_applied: query.filters,
        ranking_method: "cosine",
        result_details: [ for res in results ->
            ExplainResultDetail{
                chunk_id: res.chunk_id, rank: res.rank,
                vector_score: res.score, keyword_score: None, fused_score: res.score,
                eligibility_reason: reason(res, query.filters) } ] }   # e.g. "knn top-K; passed project+tag filter"
```

`reason(...)` states why the chunk survived: its knn position and which filters it passed —
the observability artefact that makes retrieval debuggable (PRD §7).

---

## `print_results(run, query)`   (formatter — presentation only)

```
FN print_results(run, query):
    print "query : \"{query.text}\"   mode=vector  top-k={query.top_k}"
    print "filters: {render(query.filters) or '(none)'}"            # AC-4 echo active filters (R3)
    IF run.reason == INDEX_EMPTY:
        print "index is empty — run `tovli ingest <folder>` first"; RETURN     # AC-8
    IF run.results is empty:
        print "no results" + (query.filters.is_empty() ? "" : " for these filters"); RETURN  # AC-5/E4
    FOR res IN run.results:                                          # AC-1/2
        print "#{res.rank}  score={res.score:.4f}  {res.source_path}"
        print "      {res.heading_path joined ' > '}"
        print "      {first_line(res.preview)}   [{res.chunk_id}]"
    print "latency: {run.latency_ms} ms   below-threshold: {run.below_threshold_count}"
    IF run.explain: print_explain(run.explain)                      # AC-6
```

---

## Complexity & performance (C4: 5,000 chunks < 1 s)
| Step | Cost | Note |
|------|------|------|
| Embed query | one ONNX forward pass | dominates wall-clock for the ONNX path (~tens of ms); ~0 for mock |
| `vector_search` (HNSW knn) | `O(C · log N)`, `C = K·F` | `F=5`, `K=8`, `N=5000` → trivial (<< 1 s) |
| `lookup.find_many` | `O(D)` redb point reads, `D ≤ C` | tiny; batched in one read txn |
| Filter + normalize + trim | `O(C)` | linear scan, preserves order |

Memory: only `C` candidates + `D` doc records held at once — bounded, independent of `N`.

## Determinism / testability
- `SearchService` is generic over the three ports (`Embedder`, `VectorSearchPort`, `DocumentLookupPort`)
  → unit-testable with in-memory fakes + `MockEmbedder`, no ONNX/disk (mirrors M1's orchestrator tests).
- `rrun_id()` and timestamps are injected (like M1's `now`) so runs are reproducible in tests.

## Phase-2 Gate (criteria for advancing to Architecture)
- [x] Pseudocode covers all acceptance criteria — AC-1…AC-8 annotated inline at the steps that satisfy them.
- [x] Error paths explicit — empty/blank query (E1/E2), empty index (E3/AC-8), filter-empty (E4/AC-5), model mismatch (E8/AC-7), orphan/deleted docs (E9), unsupported mode.
- [x] Complexity annotated — per-step table above; confirms C4 (< 1 s @ 5k chunks).
- [x] The three surfaced risks have algorithmic handling — over-fetch (E5), document-join filtering (E6), score normalization (E7).

**Gate result: PASS.** → Phase 3 (Architecture).
