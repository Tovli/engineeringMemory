# ADR-0007: Versioned prompt templates in source, stamped into every answer log

- **Status:** Accepted
- **Date:** 2026-06-06
- **Milestone:** M4 — RAG Answer Generation
- **Context refs:** PRD §8.6 FR-RAG-004, `docs/ddd/contexts/answer-generation.md` (`PromptTemplate`, `PromptTemplateRepository`), ADR-0006, `src/answer_generation/domain/prompt_template.rs`

## Context
FR-RAG-004 requires that answer-generation prompts are **versioned**, that templates **live in source
control**, that each answer log **stores the prompt version**, and that prompt changes can be
**evaluated against the test set**. The DDD model describes a `PromptTemplate` value object (with a
`version`) and a `PromptTemplateRepository` (`findByVersion` / `findActive` / `save`). The open
question (spec D-PROMPT-VERSION) is *how* templates are stored and the version assigned: a database/
file-backed repository, or something lighter for a local-first CLI at M4 scale.

## Decision
- **Templates are const-in-source.** `prompt_template::active()` returns a `PromptTemplate { version,
  system_prompt, context_template, instructions }`, with `ACTIVE_VERSION` a `const &str` (currently
  `"v1.0.0"`). The strings live in `src/answer_generation/domain/prompt_template.rs` — i.e. **in source
  control**, satisfying FR-RAG-004 directly, with no DB/file I/O.
- **The version is immutable per content.** Changing any prompt string requires bumping `ACTIVE_VERSION`
  (semver). The value object carries its own `version` so callers never assume "active == some fixed id".
- **Every Answer stamps the version** — including no-answer responses — in `Answer.prompt_template_version`,
  serialized into the JSONL answer log. This makes the log **regression-queryable by prompt version**
  (the DDD `findByPromptVersion` use case) without a repository abstraction yet.
- **The `PromptTemplateRepository` is deferred.** A file/DB-backed registry of multiple historical
  versions is not needed at M4; `active()` is the single source. The repository can be added later
  behind the same `PromptTemplate` type without touching `RagAnswerService`.

## Consequences
- **+** Simplest thing that fully satisfies FR-RAG-004: templates are versioned, in source control, and
  the version is recorded per answer — with zero new infrastructure.
- **+** Prompt-change evaluation is real: re-run the eval/ask flow after a version bump and diff answer
  logs filtered by `promptTemplateVersion`.
- **+** `RagAnswerService` depends only on the `PromptTemplate` value object, so swapping `active()` for a
  repository later is a non-breaking change.
- **−** Only the **active** version is reachable at runtime; you cannot ask the binary to render an old
  version without checking out that commit. Acceptable at M4 (the binary is the prompt's home); revisit
  if A/B prompt comparison in a single run is needed.
- **−** Discipline-dependent: forgetting to bump `ACTIVE_VERSION` on a prompt edit makes two different
  prompts share a version. Mitigated by the version living next to the strings it labels.
- **Future:** when a `PromptTemplateRepository` lands (multi-version store), `active()` becomes
  `repo.find_active()` and logs already carry the version needed to look templates up.
