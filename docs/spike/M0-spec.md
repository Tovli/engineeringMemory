# M0 Spike — Specification (SPARC · Specification phase)

**Goal:** Prove that RuVector can be embedded and queried from Rust on this machine —
insert vectors with metadata, run a k-NN similarity query, and get back the correct
nearest neighbours with scores. No Docker, no Postgres, no network.

## Engine decision (resolves PRD §16 Decision #1)
- Use the **`ruvector-core`** crate (crates.io v2.2.0) embedded in-process.
- Features: `storage,hnsw,parallel` only — pure-Rust subset (no `simd`/simsimd C build).
- `ruvector-postgres` (the pgrx extension + Docker) is explicitly **deferred** ("add later").

## Deterministic dataset
6 toy "document" vectors (dim = 4) in 3 clearly separated topic clusters, so the
correct ordering is obvious by inspection and does not depend on a real embedding model.

| id            | vector              | topic        |
|---------------|---------------------|--------------|
| doc-arch-1    | [1.0, 0.0, 0.0, 0.1] | architecture |
| doc-arch-2    | [0.9, 0.1, 0.0, 0.0] | architecture |
| doc-deploy-1  | [0.0, 1.0, 0.0, 0.1] | deployment   |
| doc-deploy-2  | [0.1, 0.9, 0.0, 0.0] | deployment   |
| doc-auth-1    | [0.0, 0.0, 1.0, 0.1] | auth         |
| doc-auth-2    | [0.0, 0.1, 0.9, 0.0] | auth         |

Each entry carries metadata: `title`, `topic`, `source` (mirrors the PRD chunk metadata).

Distance metric: **Cosine**. Quantization: **None** (keep distances exact for verification).

## Query & expected result
Query vector: `[0.95, 0.05, 0.0, 0.0]` (an "architecture-flavoured" question), k = 3.

Hand-computed cosine similarity (higher = closer → lower cosine *distance*):
- doc-arch-2 ≈ 0.9984  ← nearest
- doc-arch-1 ≈ 0.9936
- both deployment/auth docs ≈ 0 (far)

## Success criteria (automated PASS/FAIL in the spike)
1. `count()` returns 6 after ingest.
2. Top-1 result id == `doc-arch-2`.
3. Top-2 result id == `doc-arch-1`.
4. Both top-2 hits have `topic == "architecture"`.
5. Scores are non-decreasing (confirms "lower distance = better" orientation).
6. Query latency is printed (NFR observability).

If all hold, the spike PASSES and M0's core acceptance ("insert vectors + similarity
search, locally") is met.

## Architecture seam (PRD §15 Risk-1)
All RuVector calls live behind a `VectorStore` trait (`src/vector_store.rs`). `main.rs`
contains no RuVector-specific logic — it builds sample data, calls the trait, prints,
and self-checks. This is the seam the future `VectorStoreService` / Postgres backend
will slot into.
