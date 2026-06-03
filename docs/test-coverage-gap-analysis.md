# Test Coverage Gap Analysis — tovli

**Date:** 2026-06-03
**Branch:** `tests/covering-gaps`
**Scope:** `src/` (3,683 LOC across 44 Rust files), milestones M0–M3
**Method:** Static test-to-code mapping (see caveat below)

---

## ⚠️ Methodology caveat

This is **not** instrumented line/branch coverage. No coverage tool is wired into the
project (`cargo-llvm-cov` / `tarpaulin` not present), so percentages here are **structural
estimates** derived by mapping each module's branches against the 51 test functions that
exercise them — not measured execution counts.

To get ground-truth numbers, run:

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --lib --tests --html        # default features (no ONNX)
```

> Note (per project memory): the Windows GNU toolchain needs WinLibs `dlltool` on PATH and
> the pure-Rust ruvector feature subset. `cargo-llvm-cov` uses LLVM source-based coverage and
> should work under the same toolchain that already builds the crate.

---

## Executive summary

| Metric | Value |
|---|---|
| Source files | 44 (`src/`) |
| Total LOC | 3,683 |
| Test functions | **51** (46 inline unit + 5 integration) |
| Modules with **inline** unit tests | 12 / 44 |
| Modules with **zero** automated coverage | 3 (`main.rs`, `vector_store.rs`, `onnx_embedder.rs`) |
| Estimated line coverage | **~70–75%** (application + domain strong; infra/CLI weak) |

**Headline finding:** The **application and domain layers are well covered** (search ranking,
metrics math, relevance, chunking, filters — 46 inline tests with fakes). The gaps cluster in
two predictable places:

1. **Infra adapters** — only the *happy path* is hit by the 5 integration tests; their
   error/edge branches (missing files, malformed JSON, dim mismatch, delete/merge paths) are
   **untested**.
2. **The CLI (`main.rs`, 455 LOC) and the M0 spike store (`vector_store.rs`, 132 LOC)** —
   **no automated tests at all**.

---

## Coverage map by module

Legend: ✅ unit-tested · 🔶 happy-path only (via integration) · ❌ untested

### Application layer — **strong**

| Module | LOC | Tests | Status |
|---|---|---|---|
| `retrieval/application/search_service.rs` | 407 | 9 | ✅ |
| `retrieval/application/filters.rs` | 109 | 6 | ✅ |
| `retrieval/application/scoring.rs` | 44 | 2 | ✅ |
| `evaluation/application/metrics_calc.rs` | 116 | 4 | ✅ |
| `evaluation/application/evaluation_service.rs` | 229 | 4 | ✅ |
| `evaluation/application/relevance.rs` | 90 | 3 | ✅ |
| `ingestion/orchestrator.rs` | 393 | 6 | ✅ |
| `ingestion/chunking.rs` | 336 | 4 | ✅ |

### Domain layer — **mixed** (mostly data; small logic gaps)

| Module | LOC | Tests | Status | Untested logic |
|---|---|---|---|---|
| `ingestion/domain.rs` | 146 | 2 | ✅ | — |
| `retrieval/domain/errors.rs` | 32 | 0 | ❌ | `RetrievalError` **Display** — the mismatch message (user-facing remediation text) is unverified |
| `retrieval/domain/query.rs` | 44 | 0 | 🔶 | `MetadataFilter::is_empty` (only hit indirectly) |
| `evaluation/domain/question.rs` | 25 | 0 | ❌ | `EvalQuestion::has_ground_truth` |
| other domain structs | — | 0 | 🔶 | plain data / serde — low risk |

### Infra layer — **happy-path only** (the main gap)

| Module | LOC | Tests | Status | Untested branches |
|---|---|---|---|---|
| `ingestion/infra/ruvector_store.rs` | 113 | 0 | 🔶 | **dim-mismatch `bail!`**, `delete_by_document`, chunk-id **append/merge on re-upsert**, `read_chunk_ids` when table missing |
| `retrieval/infra/redb_lookup.rs` | 82 | 0 | 🔶 | missing-file → `None` (AC-8), `TableDoesNotExist`, **deleted-doc filtering** in `indexed_model_version`, `find_many` subset, malformed JSON |
| `ingestion/infra/redb_repo.rs` | 78 | 0 | 🔶 | **`soft_delete`** (id→record lookup), `active_under` root prefix filter, `find_by_path` miss |
| `retrieval/infra/ruvector_search.rs` | 70 | 0 | 🔶 | metadata parsing: `heading_path` split, empty metadata, non-string values, missing keys |
| `evaluation/infra/dataset_loader.rs` | 59 | 3 | ✅ | — |
| `evaluation/infra/report_writer.rs` | 47 | 0 | 🔶 | parent-dir creation, empty-parent branch (shape checked by integration test) |
| `ingestion/embedding.rs` (`EmbeddingService`) | 42 | 0 | 🔶 | **count-mismatch `bail!`**, **dim-mismatch `bail!`**, empty-chunks early return |
| `evaluation/infra/retrieval_search_adapter.rs` | 20 | 0 | 🔶 | trivial delegation — low risk |

### Zero coverage — **no test touches these**

| Module | LOC | Status | Notes |
|---|---|---|---|
| `main.rs` | 455 | ❌ | Entire CLI: arg parsing, `mode != "vector"` → `exit(2)`, `build_embedder`, `render_filters`, all `print_*` formatting, eval exit codes (`Failed`/`ThresholdFailed` → `exit(1)`), `run_spike` self-checks |
| `vector_store.rs` | 132 | ❌ | M0 spike store (`RuVectorStore`): `upsert`/`query`/`count`/`metadata_of` — only run via `tovli spike`, never asserted automatically |
| `ingestion/infra/onnx_embedder.rs` | 59 | ❌ | Feature-gated (`--features onnx`); local-only by design — **acceptable gap**, documented |

---

## Risk-weighted prioritization

Risk = (likelihood a bug hides in the untested branch) × (blast radius if it ships). Infra
data-corruption and silent-wrong-results paths rank highest; the spike store ranks lowest.

### 🔴 P0 — data integrity & silent-wrong-results (do first)

1. **`ruvector_store.rs` — `delete_by_document` + chunk-id merge.**
   Re-ingesting a changed file deletes old vectors and appends new chunk ids. A bug here
   leaves **orphaned vectors** (stale search hits) or **deletes the wrong chunks**. The
   append/merge on re-upsert is logic, not glue — and it is completely untested.
   *Suggested:* unit test with a temp redb — upsert doc, re-upsert with new chunks, assert
   old chunk ids gone from the map and vectors deleted; assert dim-mismatch `bail!`.

2. **`redb_lookup.rs` — `indexed_model_version` deleted-doc filtering.**
   This drives the **embedding-model compatibility guard (R6/AC-7)** and the empty-index
   path (AC-8). If a deleted doc leaks through, search opens the store with the wrong
   dimension. Untested: deleted-doc skip, missing-file→`None`, `TableDoesNotExist`.

3. **`embedding.rs` — dimension/count invariant `bail!`s (FR-EMB-001/AC-8).**
   The dimension invariant is a stated PRD guarantee. Both guard branches are untested at
   the unit level. Cheap to cover with a stub `Embedder` that returns wrong-length vectors.

### 🟠 P1 — correctness of supporting paths

4. **`redb_repo.rs` — `soft_delete` and `active_under`.**
   `soft_delete` looks a record up *by id* (not path-key) and flips status — an O(n) scan
   with a subtle key/id mismatch risk. `active_under` is the root-prefix filter that decides
   which docs get re-scanned. Both untested.

5. **`ruvector_search.rs` — metadata extraction.**
   `heading_path` round-trips through a `" > "` join/split; non-string metadata values are
   silently dropped. Edge cases (empty heading, missing keys, numeric metadata) untested.

6. **`retrieval/domain/errors.rs` — `RetrievalError` Display.**
   The mismatch message contains user-facing remediation (`tovli reembed --model …`). A
   formatting regression ships bad guidance. One-line assertion test.

### 🟡 P2 — UX / formatting / low-blast-radius

7. **`main.rs` CLI behavior.** Pure I/O shell, but untested exit codes matter for the **CI
   regression gate** (`ThresholdFailed` → `exit(1)`, bad mode → `exit(2)`). Best covered by
   a few `assert_cmd`-style integration tests against the built binary rather than unit tests.

8. **`report_writer.rs` parent-dir creation** — minor; integration test already checks shape.

9. **`vector_store.rs` (M0 spike).** Legacy demo path. Low priority unless it becomes a real
   backend; the `tovli spike` self-checks act as a manual smoke test today.

10. **`evaluation/domain/question.rs::has_ground_truth`** — trivial predicate; bundle into an
    existing domain test.

---

## Recommended next steps

1. **Wire up real coverage first** — `cargo llvm-cov --lib --tests` — to replace the estimates
   in this doc with measured numbers and confirm the gap map.
2. **Knock out P0 (items 1–3)** — three focused unit tests against temp redb files / stub
   embedders. Highest risk-reduction per line of test code; targets data-corruption and the
   silent-wrong-dimension failure mode.
3. **P1 (items 4–6)** — adapter edge-case tests; small and fast.
4. **P2 CLI exit codes** — add `tests/cli.rs` with `assert_cmd` covering the `exit(1)`/`exit(2)`
   paths that gate CI.

**Projected impact:** P0 + P1 (~6 small test modules) would move infra from 🔶 happy-path-only
to ✅ on the branches that can actually corrupt the index or return wrong results — the highest-
value coverage this codebase is currently missing.
