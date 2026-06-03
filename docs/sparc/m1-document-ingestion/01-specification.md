# SPARC Phase 1 — Specification: M1 Document Ingestion

Feature: **M1 — Document Ingestion** · Slug: `m1-document-ingestion` · Started: 2026-06-02
Derived from PRD §8.1 (FR-ING-*), §8.2 (FR-CHK-*), §8.3 (FR-EMB-*), §11 (data model), §12.2 (CLI).
Adapted to the **Rust + `ruvector-core`** stack established in M0 (reuses the `VectorStore` seam).

## Goal
`tovli ingest <folder>` scans a folder, parses supported files, chunks them, hashes content,
generates embeddings, and stores documents + chunks + embeddings in RuVector — idempotently
and incrementally — printing a clear summary.

## Requirements
- **R1** (FR-ING-001) Recursively scan a folder and ingest supported documents.
- **R2** (FR-ING-002) Support `.md .txt .json .yaml .yml`; skip others with a warning; parsing isolated behind a `Parser` trait.
- **R3** (FR-ING-003) Idempotent: content hash per file + per chunk; re-ingesting unchanged files creates no duplicate chunks.
- **R4** (FR-ING-004) Incremental: add new, reprocess modified, mark deleted/removed, skip unchanged.
- **R5** (FR-ING-005) Extract metadata: path, name, ext, content hash, created/updated ts, chunk index, char length, optional title/tags/project/topic.
- **R6** (FR-CHK-001) Markdown-aware chunking by headings; preserve heading path; never split a fenced code block; avoid splitting tables.
- **R7** (FR-CHK-002) Configurable chunk size (defaults: target 500 / max 800 / overlap 80 tokens); report config used.
- **R8** (FR-CHK-003) Deterministic chunk preview, no LLM.
- **R9** (FR-EMB-001) Embedding provider behind a trait; record provider + model name + dimension; prevent mixed-dimension indexes.
- **R10** (FR-EMB-002) Persist `embedding_model`, `embedding_dimension`, `embedding_created_at` per chunk.
- **R11** Store via the existing `VectorStore` seam (RuVector backend); no new coupling to the engine.
- **R12** (CLI §12.2) `tovli ingest <path>` with `--force`, `--dry-run`, `--project <p>`, `--tag <t>`; print an ingestion summary. CLI delegates to an `IngestionService` (no logic in the handler, PRD §9.4).

## Acceptance Criteria
- **AC-1** `tovli ingest ./docs` over ≥20 markdown files creates document + chunk records, stores embeddings, and prints counts (scanned / ingested / skipped / chunks).
- **AC-2** Unsupported files (e.g. `.png`) are skipped and listed in the summary with a warning; supported types are parsed.
- **AC-3** Running ingest twice with no changes creates **zero** new chunks; summary reports `unchanged: N` (idempotency via content hash).
- **AC-4** Modifying one file and re-running re-chunks **only** that file; others are skipped.
- **AC-5** Markdown chunking preserves the heading path and **never** splits a fenced ```code``` block.
- **AC-6** Each chunk persists: content hash, chunk index, char length, heading path, embedding model name + dimension.
- **AC-7** `--dry-run` reports what would be ingested **without** writing to the store.
- **AC-8** Inserting a chunk whose embedding dimension differs from the index dimension fails with a clear, actionable error.

## Constraints
- **C1** Rust (edition 2021), builds under GNU toolchain + WinLibs, **no Docker**. Reuse the `VectorStore` seam from M0.
- **C2** Local-first / privacy (NFR §9.2): the **default** embedding provider must run offline; no data leaves the machine by default.
- **C3** Keep `ruvector-core` on the pure-Rust feature subset unless the chosen embedding provider requires more (e.g. `onnx-embeddings` pulls `ort` — a Windows build risk to settle in Architecture).
- **C4** Performance (NFR §9.1): ingest 100 markdown files in < 5 minutes locally.
- **C5** Clippy-clean; CLI handlers contain no ingestion/engine logic (delegate to a service).
- **C6** Idempotency via deterministic content hashing; deterministic chunk IDs.

## Edge Cases
- **E1** Empty / whitespace-only file → 0 chunks, not an error.
- **E2** Single fenced code block exceeding `maxChunkTokens` → split safely without breaking the fence (or keep whole + flag).
- **E3** Non-UTF8 / binary file with a supported extension → skip with warning, don't panic.
- **E4** Identical content in two files → distinct documents; chunk-hash dedup while preserving provenance.
- **E5** File deleted between runs → mark deleted / remove from active index (R4).
- **E6** Symlink loops / very deep trees → recursion guard.
- **E7** No real tokenizer yet → token counting needs a strategy (heuristic vs tokenizer crate) — see D-TOK.
- **E8** Concurrent ingest runs → out of scope for M1 (assume single run).

## Open Decisions (resolve in Architecture / interactively)
- **D-EMB** → **RESOLVED: Local ONNX (MiniLM)** via `ruvector-core` `onnx-embeddings`. Offline + privacy-aligned (C2). **Consequence:** `ruvector-core` pins `ort = 2.0.0-rc.9`; ONNX Runtime prebuilt binaries are **MSVC-only** on Windows (no `x86_64-pc-windows-gnu` prebuilts), and `tokenizers` uses the `onig` C feature. → **New constraint C7: build under the `x86_64-pc-windows-msvc` toolchain** (VS Build Tools + Windows SDK), replacing the M0 GNU toolchain. The mock provider still exists behind the trait for tests (FR-EMB-001).
- **D-TOK** Token counting: real tokenizer crate (`tokenizers`) vs char/word heuristic for chunk sizing.
- **D-HASH** Content hash: `blake3` (fast) vs `sha256`.
- **D-PERSIST** Document/incremental-state storage: RuVector metadata only, or a sidecar store (redb/sqlite) for `documents` records + change tracking (the PRD data model implies separate document/chunk tables).

## Phase-1 Gate (criteria for advancing to Pseudocode)
- [x] ≥ 3 acceptance criteria → **8 defined**
- [x] Explicit constraints → **6 defined**
- [x] Edge cases identified → **8 defined**

**Gate status: ready to PASS.** Advancing to Phase 2 (Pseudocode) requires confirming the
open decisions are acceptable to defer (D-EMB in particular shapes the architecture).
