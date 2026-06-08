# ADR-0008: Citation protocol (chunk-id tags) and the no-answer / exit-code policy

- **Status:** Accepted
- **Date:** 2026-06-06
- **Milestone:** M4 — RAG Answer Generation
- **Context refs:** PRD §8.6 FR-RAG-002/003, §7 ("Sources Are Mandatory"), §15 Risk 3, `docs/ddd/contexts/answer-generation.md` (no-answer flow, `LlmPort`), ADR-0006, `src/answer_generation/application/rag_service.rs`

## Context
ADR-0006 fixed *that* the LLM sits behind `LlmPort` and *that* the domain enforces grounding. M4
implementation forces three concrete questions the spec left open:

1. **D-CITATION-PROTOCOL** — by what wire format does the LLM tell us which chunks it used, so we can
   validate citations against the RetrievalRun (FR-RAG-002, no invented citations)?
2. **D-CONFLICT** — FR-RAG-003 lists `sourcesConflict` as a no-answer reason. Do we detect contradiction
   in M4?
3. **D-EXITCODE** — a no-answer is a legitimate product response. What exit code does `tovli ask` return
   for each outcome, so scripts/CI can tell "no reliable source" apart from "provider broken"?

## Decision
**Citation protocol.** The context renderer tags each chunk with a machine-readable id:
`[[chunk:<chunk_id>]] (source=<path>) [<headings>]\n<text>`. The prompt instructs the model to end its
reply with `SOURCES: <comma-separated chunk ids>`. The adapter parses that line into
`LlmResponse.cited_chunk_ids`. `RagAnswerService` then **validates** those ids against the RetrievalRun:
- ids not present in the run are **stripped** (no invented citations);
- if **no** valid id remains, the answer becomes `noAnswerReason = outsideCorpus` (fail closed);
- citations are rebuilt from the run's results in rank order, and every other retrieved chunk is recorded
  in `retrievedButUnusedChunks`.
The `MockLlm` implements the protocol from the model side by echoing the `[[chunk:…]]` tags it was given,
so the default/offline path is deterministic and exercises the same validation code.

**D-CONFLICT — deferred.** `SourcesConflict` exists in `NoAnswerReason` (so the wire format is stable),
but M4 does **not** detect contradiction: reliable conflict detection needs a richer LLM contract
(structured conflict signal) and its own evaluation. Contradictory chunks fall through to a normal cited
answer — never a silent merge that hides the conflict.

**D-EXITCODE.** `tovli ask` exit codes:
- **0** — a grounded answer **or** a no-answer for `belowSimilarityThreshold` / `outsideCorpus`
  (these are valid product responses: "here's the answer" / "I have no reliable source").
- **2** — bad invocation (e.g. `--mode` other than `vector`), matching `search`/`eval`.
- **3** — `llmProviderError` (the provider was unavailable or failed): retrieval worked but generation
  could not run — an infra failure, distinct from "no source".

## Consequences
- **+** Hallucination (PRD §15 Risk 3) is contained by **code, not prompt trust**: the citation
  invariant holds regardless of what the model emits — invented ids can't survive validation.
- **+** The protocol is mock-friendly: the deterministic `MockLlm` round-trips the same tags, so CI
  verifies the real parse/validate path with no network.
- **+** Exit codes let CI/scripts branch correctly — a "no reliable source" answer doesn't fail a
  pipeline, but a broken provider does.
- **−** The model must follow the `SOURCES:` format; a model that ignores it yields zero valid citations
  → `outsideCorpus`. This is the intended fail-closed behavior, but it makes prompt/format adherence part
  of answer quality (tracked via the versioned prompt, ADR-0007).
- **−** `sourcesConflict` is defined but unreachable in M4 — a known gap surfaced here rather than implied.
- **Future (M6):** feedback can flag answers whose citations were wrong, feeding prompt/version iteration;
  conflict detection can light up `SourcesConflict` without changing the wire format.
