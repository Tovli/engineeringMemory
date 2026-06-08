# SPARC Phase 2 — Pseudocode: M4 RAG Answer Generation

Algorithms for the **Answer Generation** context. Annotated with the AC / edge case each step covers.
`N` = retrieved results in the run, `C` = eligible context chunks (score ≥ threshold).

---

## Top-level: `ask` command

```
FN cli_ask(args):                                      # main.rs — thin (R12, C5)
    IF args.mode != "vector": ERROR "mode available in M5; use --mode vector"; EXIT 2
    embedder = build_embedder(args.mock)               # same builder as search/eval (C2)
    query    = Query{ text: args.query, mode: Vector, filters: none,
                      top_k: args.top_k, embedding_model: embedder.model_version() }
    print_header(query)
    lookup   = open_document_lookup(read_only)
    indexed  = lookup.indexed_model_version()
    IF indexed IS None:                                # empty index → nothing to ground on
        print "index is empty — run `tovli ingest` first"; RETURN ok        # exit 0
    store    = open_vector_search(indexed.dimension)
    run      = SearchService{embedder, store, lookup}.search(query, explain=false)  # model mismatch → fatal err here

    IF args.show_context OR args.no_llm: print_context(run)                  # R6 / AC-3
    IF args.no_llm: RETURN ok                          # AC-5 retrieval-only — never build the LLM

    llm    = MockLlm::default()                        # R10; real provider = future feature-gated adapter
    answer = RagAnswerService{llm}.generate(run, {query_id, answer_id, now, max_tokens})  # core
    print_answer(answer)                               # AC-1 (answer + Sources) / FR-RAG-003 message
    append_answer_log(".tovli/answers.jsonl", answer)  # R9 / AC-4 (infra)

    IF answer.no_answer_reason == LlmProviderError: EXIT 3    # D-EXITCODE (ADR-0008)
    RETURN ok                                          # grounded OR below-threshold/outside-corpus = exit 0
```

The CLI does retrieval, then hands the **RetrievalRun** to generation — Answer Generation never queries
the store itself (PRD §7 "Retrieval Before Generation"). `--no-llm` short-circuits before the provider
is ever constructed (E8).

---

## `RagAnswerService.generate(run, ctx) -> Answer`   (application core)

```
FN generate(run, ctx):                                 # never panics — every path returns an Answer
    t0 = now(); template = prompt_template::active()   # version stamped on EVERY return (AC-4)

    # 1. Weak-retrieval gate — refuse BEFORE any LLM call (AC-2, E1/E2)
    context = assemble(run, MAX_CONTEXT_TOKENS)        # keeps score ≥ SIMILARITY_THRESHOLD only
    IF context.chunks.is_empty():
        RETURN no_answer(BelowSimilarityThreshold, "no source cleared the threshold …")

    # 2. Provider availability (E6) — never call complete() when down
    IF NOT llm.is_available():
        RETURN no_answer(LlmProviderError, "provider unavailable; retrieval succeeded …")

    # 3. Render + call
    request  = render(template, context, ctx.max_tokens)
    response = llm.complete(request)
    IF response IS Err OR response.finish_reason == Error:
        RETURN no_answer(LlmProviderError, "provider failed/errored")

    # 4. Empty text (E5)
    IF response.text.trim().is_empty():
        RETURN no_answer(OutsideCorpus, "sources don't appear to answer this")

    # 5. Validate citations against the run — strip invented ids (AC-6, E4)
    valid = { r.chunk_id for r in run.results }
    cited = dedup([ id for id in response.cited_chunk_ids IF id in valid ])

    # 6. No valid citation remains (E3, AC-6) — fail closed, never emit ungrounded prose
    IF cited.is_empty():
        RETURN no_answer(OutsideCorpus, "couldn't ground an answer in the sources")

    # 7. Build citations (rank order from the run) + retrieved-but-unused list (FR-RAG-002)
    citations = [ Citation{r.rank, r.chunk_id, r.source_path, r.heading_path, r.preview}
                  for r in run.results IF r.chunk_id in cited ]
    unused    = [ r.chunk_id for r in run.results IF r.chunk_id NOT in cited ]

    # 8. Grounded answer (AC-1, AC-7: citations non-empty ⇔ no_answer_reason is None)
    RETURN Answer{ id, query_id, query_text: run.query.text, retrieval_run_id: run.id,
                   prompt_template_version: template.version,
                   answer_text: response.text.trim(), citations,
                   retrieved_but_unused_chunks: unused, no_answer_reason: None,
                   llm_provider: response.provider, latency_ms: t0.elapsed(), created_at: ctx.now }

FN no_answer(reason, message):                          # always: version stamped, message non-empty
    RETURN Answer{ …, prompt_template_version: template.version, answer_text: message,
                   citations: [], retrieved_but_unused_chunks: all run chunk ids,
                   no_answer_reason: Some(reason), … }   # invariant 2 (user always told why)
```

**Citation invariant (AC-7):** the only way `citations` is empty is when `no_answer_reason` is `Some`.
Enforced by construction at steps 6 and 8 — independent of what the LLM emits (PRD §7, FR-RAG-002).

---

## `assemble(run, max_tokens) -> RetrievedContext`   (pure — C4, E7)

```
FN assemble(run, max_tokens):
    chunks = [ ContextChunk{r.rank, r.chunk_id, r.source_path, r.heading_path,
                            text: r.preview, r.score}
               for r in run.results IF r.score >= SIMILARITY_THRESHOLD ]   # reuse M2 const (C4)
    total = sum(estimate_tokens(c.text) for c in chunks)
    WHILE chunks.len() > 1 AND total > max_tokens:      # E7 — drop lowest-rank first, keep ≥1
        total -= estimate_tokens(chunks.pop().text)
    RETURN RetrievedContext{ query_text: run.query.text, chunks }
```

`SIMILARITY_THRESHOLD` is imported from `retrieval::application::scoring` — eval, search and RAG all
agree on "weak" (ADR-0003/0006). In M4 `text` is the result `preview` (the only chunk text the index
carries — see the completion doc); the eligibility gate, not full content, is the M4 contract.

## `render(template, context, max_tokens) -> LlmRequest`   (pure — ADR-0008 citation protocol)

```
FN render(template, context, max_tokens):
    chunks_block = join("\n\n", [ "[[chunk:{c.chunk_id}]] (source={c.source_path}) [{headings}]\n{c.text}"
                                  for c in context.chunks ])
    body = template.context_template.replace("{{chunks}}", chunks_block)
                                    .replace("{{question}}", context.query_text)
    RETURN LlmRequest{ system_prompt: template.system_prompt,
                       user_message: body + "\n\n" + template.instructions, max_tokens }
```

The `[[chunk:<id>]]` tag is the wire format of the citation protocol (ADR-0008): the model is told to
echo the ids it used on a `SOURCES:` line; the adapter parses that line back into `cited_chunk_ids`.

---

## `MockLlm` (infra, default provider — ADR-0006)

```
FN MockLlm.is_available(): RETURN true
FN MockLlm.complete(request):
    ids   = parse_chunk_ids(request.user_message)       # scan for [[chunk:ID]] tags, in order, dedup
    cited = ids.take(max_citations)
    text  = cited.is_empty() ? "" : "Based on the retrieved documentation … grounded in {n} source(s)."
    RETURN LlmResponse{ text, cited_chunk_ids: cited, finish_reason: Stop, provider: "mock-llm", latency_ms: 0 }
```

Deterministic and offline: it cites the tags the renderer emitted, so the validate-against-run step
always passes for the mock → reproducible grounded answers in CI. A real provider implements the same
`LlmPort` and parses the model's real `SOURCES:` line.

## `append_answer_log(path, answer)`   (infra, R9/AC-4) · `print_answer` / `print_context` (presentation)

```
FN append_answer_log(path, answer):                     # JSONL — accumulates across runs
    mkdir_parents(path); append_line(path, json(answer))    # camelCase; promptTemplateVersion always present

FN print_answer(answer):
    print answer.answer_text
    IF answer.no_answer_reason: print "(no answer: {reason})"
    ELSE: print "Sources:"; FOR i,c IN answer.citations: print "{i+1}. {c.source_path}#{c.chunk_id}"
    print "prompt: {answer.prompt_template_version}   provider: {answer.llm_provider}"
```

---

## Complexity & performance (C9)
| Step | Cost | Note |
|------|------|------|
| retrieval (one search) | embed + HNSW knn | same as M2; dominates with a real LLM out of the picture |
| assemble + render | `O(N)` | tiny |
| `llm.complete` | one provider round-trip (0 for mock) | the real cost with a cloud/local LLM |
| validate citations | `O(N)` over a set | trivial |
| append_answer_log | one append | trivial |

`--no-llm` removes the only heavy step (the LLM call), so retrieval-only is as fast as `search`.

## Determinism / testability
- `RagAnswerService` is generic over `LlmPort` → unit-tested with **fake** LLMs (canned text/citations,
  availability, finish reason) — every no-answer branch verified with zero I/O.
- `assemble` / `render` are pure → directly unit-tested (threshold filter E2, budget trim E7, tag format).
- `MockLlm` is deterministic → the integration test gets a reproducible grounded answer (ADR-0005 trick:
  query text equals a chunk's content → mock embeds identically → similarity ≈ 1.0 clears the threshold).
- `query_id` / `answer_id` / `now` injected → reproducible answer logs in tests.

## Phase-2 Gate (criteria for advancing to Architecture)
- [x] Pseudocode covers all acceptance criteria — AC-1…AC-8 annotated at the steps that satisfy them.
- [x] Error paths explicit — empty/weak retrieval (E1/E2), provider down/error (E6), empty text (E5),
      invented/zero citations (E3/E4), token-budget trim (E7), `--no-llm` builds no provider (E8).
- [x] Complexity annotated — per-step table; the LLM call is the only non-trivial cost (C9).
- [x] Reuses M2 unchanged (one `SearchService.search`) + the shared `SIMILARITY_THRESHOLD` constant (C4/C6).

**Gate result: PASS.** → Phase 3 (Architecture).
