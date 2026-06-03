# SPARC Phase 2 — Pseudocode: M3 Retrieval Evaluation

Algorithms for the **Evaluation** context. Annotated with the AC / edge case each step covers.
`Q` = number of questions, `K` = effective retrieval depth (`max(topK, 5)`).

---

## Top-level: `eval` command

```
FN cli_eval(args):                                     # main.rs — thin (R9, C5)
    IF args.mode != "vector": ERROR "mode available in M5; use --mode vector"; EXIT 2
    questions = load_dataset(args.path)                # FR-EVAL-001; E1/E2 validated here
    embedder  = build_embedder(args.mock)              # same builder as search (C2)
    lookup    = open_document_lookup(read_only)
    indexed   = lookup.indexed_model_version()
    IF indexed IS None:                                # E9 empty index
        log "index empty — run `tovli ingest` first"   # eval still runs → all-empty metrics
    store     = open_vector_search(indexed?.dimension or embedder.dim)
    search_port = RetrievalSearchAdapter{ SearchService{embedder, store, lookup} }

    config = EvalRunConfig{ mode: Vector, top_k: args.top_k,
                            threshold: args.fail_below_hit_at_3.map(minHitAt3),
                            embedding_model: embedder.model_version() }
    run = EvaluationService{search_port}.run(questions, config, run_id, now)   # core

    print_metrics(run.metrics)                         # AC-2/3/4
    write_report(args.output, run)                     # AC-5  (infra)
    IF run.status == THRESHOLD_FAILED: EXIT 1          # AC-6 / FR-EVAL-003
    EXIT 0
```

---

## `load_dataset(path) -> [EvalQuestion]`   (infra, FR-EVAL-001)

```
FN load_dataset(path):
    text = read_file(path)            ! ERROR(E2) "cannot read dataset {path}"
    qs   = json_parse::<[EvalQuestion]>(text)   ! ERROR(E2) "malformed dataset: {e}"
    IF qs.is_empty(): ERROR "dataset is empty"
    FOR q IN qs:
        IF q.expectedChunkIds.is_empty() AND q.expectedSourceFiles.is_empty():
            ERROR(E1) "question {q.id} has no expected chunks or source files"
    RETURN qs
```

`EvalQuestion` deserializes camelCase JSON: `id`, `question`, `expectedChunkIds?`, `expectedSourceFiles?`.

---

## `EvaluationService.run(questions, config, run_id, now) -> EvalRun`   (application core)

```
FN run(questions, config, run_id, now):
    k = max(config.top_k, 5)                           # E5 / ADR-0005 — enough for Hit@5
    results = []
    FOR (i, q) IN enumerate(questions):
        query = Query{ text: q.question, mode: config.mode,
                       filters: none, top_k: k,
                       embedding_model: config.embedding_model }
        run_or_err = search_port.search(query)         # R8 conformist call into Retrieval
        MATCH run_or_err:
            Err(EmbeddingModelMismatch) => RETURN EvalRun{ status: FAILED, error, ... }   # E6 fatal
            Err(other)                  => RETURN EvalRun{ status: FAILED, error, ... }
            Ok(rrun) =>
                results.push(judge(q, rrun, i, run_id))
    metrics = compute_metrics(results)                 # pure (MetricsCalculationService)
    status  = threshold_status(metrics, config.threshold)   # COMPLETED | THRESHOLD_FAILED
    RETURN EvalRun{ id: run_id, dataset, mode, top_k: config.top_k, model,
                    status, metrics, question_results: results,
                    started_at: now, completed_at: now }
```

### `judge(q, rrun, i, run_id) -> EvalQuestionResult`   (uses relevance, ADR-0004)

```
FN judge(q, rrun, i, run_id):
    returned_chunks  = [r.chunk_id    for r in rrun.results]   # rank order
    returned_sources = [r.source_path for r in rrun.results]
    first_rank = None
    FOR r IN rrun.results:                              # r.rank is 1-based, ascending
        IF is_relevant(q, r):                           # ADR-0004
            first_rank = r.rank; BREAK                  # E7 — first relevant only
    rr = first_rank ? 1.0 / first_rank : 0.0            # R4 reciprocal rank
    RETURN EvalQuestionResult{
        question_id: q.id, question_text: q.question,
        retrieval_run_id: "{run_id}_q{i}",
        returned_chunk_ids: returned_chunks, returned_source_paths: returned_sources,
        hit_at_1: first_rank.map(<=1).or(false),
        hit_at_3: first_rank.map(<=3).or(false),
        hit_at_5: first_rank.map(<=5).or(false),
        reciprocal_rank: rr,
        latency_ms: rrun.latency_ms,
        top_score: rrun.results.first().map(.score),
        empty: rrun.results.is_empty() }                # E3
```

### `is_relevant(q, result) -> bool`   (pure — ADR-0004, AC-7, E4)

```
FN is_relevant(q, result):
    IF result.chunk_id IN q.expectedChunkIds: RETURN true          # exact id match
    FOR exp IN q.expectedSourceFiles:                              # path-tolerant
        IF source_matches(result.source_path, exp): RETURN true
    RETURN false

FN source_matches(indexed_path, expected):                        # E4
    a = normalize(indexed_path)   # backslashes→/, strip leading "./", lowercase
    b = normalize(expected)
    RETURN a == b OR a.ends_with("/" + b) OR basename(a) == basename(b)
```

---

## `compute_metrics(results) -> EvalMetrics`   (pure, FR-EVAL-002, R3/R4/R5)

```
FN compute_metrics(results):
    n = results.len()
    IF n == 0: RETURN zeroed metrics
    RETURN EvalMetrics{
        hit_at_1: mean(r.hit_at_1 ? 1 : 0),
        hit_at_3: mean(r.hit_at_3 ? 1 : 0),
        hit_at_5: mean(r.hit_at_5 ? 1 : 0),
        mrr:      mean(r.reciprocal_rank),
        avg_latency_ms: mean(r.latency_ms),
        empty_result_count: count(r.empty),
        below_threshold_count: count(r.top_score.is_none() OR r.top_score < SIMILARITY_THRESHOLD),  # reuse M2 const
        question_count: n }
```

`SIMILARITY_THRESHOLD` is reused from `retrieval::application::scoring` (single source of truth).

### `threshold_status(metrics, threshold) -> EvalRunStatus`   (R7, AC-6, E8)

```
FN threshold_status(metrics, threshold):
    IF threshold AND metrics.hit_at_3 < threshold.min_hit_at_3:   # strict < — equal passes (E8)
        RETURN THRESHOLD_FAILED
    RETURN COMPLETED
```

---

## `write_report(path, run)` + `print_metrics(metrics)`   (infra / presentation, AC-5/R6/R10)

```
FN write_report(path, run):                            # serde_json pretty
    mkdir_parents(path)
    write_json(path, EvalReport{ run_id, generated_at, search_mode, top_k,
                                 embedding_model, metrics, question_results })

FN print_metrics(m):
    print "questions      : {m.question_count}"
    print "Hit@1/3/5      : {m.hit_at_1:.2} / {m.hit_at_3:.2} / {m.hit_at_5:.2}"
    print "MRR            : {m.mrr:.3}"
    print "avg latency    : {m.avg_latency_ms:.1} ms"
    print "empty results  : {m.empty_result_count}"
    print "below threshold: {m.below_threshold_count}"
```

---

## Complexity & performance (C4: 50 questions < 2 min)
| Step | Cost | Note |
|------|------|------|
| per question: one `search` | embed (ONNX ~tens ms) + HNSW knn | dominates; 50 × ~tens ms ≪ 2 min |
| judge + relevance | `O(K · E)` per q (`E` expected entries) | tiny |
| compute_metrics | `O(Q)` | trivial |
| write_report | one file write | trivial |

Serial execution is well within budget; no parallelism needed for M3.

## Determinism / testability
- `EvaluationService` is generic over a `SearchPort` → unit-tested with a **fake** returning canned
  `RetrievalRun`s, so Hit@K / MRR / threshold math is verified with zero I/O and zero embedder.
- `is_relevant` / `source_matches` and `compute_metrics` are pure → directly unit-tested (E4, E7, E8).
- `run_id` + `now` injected → reproducible reports in tests.
- Integration test: real ingest → eval with the **mock** embedder over a crafted dataset whose
  question text equals a chunk's content (mock embeds identically → that chunk ranks #1 → Hit@1 = 1.0).

## Phase-2 Gate (criteria for advancing to Architecture)
- [x] Pseudocode covers all acceptance criteria — AC-1…AC-8 annotated at the steps that satisfy them.
- [x] Error paths explicit — bad/empty dataset (E1/E2), zero results (E3), path-form mismatch (E4),
      shallow top-k (E5), fatal model mismatch (E6), threshold equality (E8), empty index (E9).
- [x] Complexity annotated — per-step table; confirms C4 (< 2 min @ 50 questions).
- [x] Reuses M2 unchanged via `SearchPort` + the shared `SIMILARITY_THRESHOLD` constant.

**Gate result: PASS.** → Phase 3 (Architecture).
