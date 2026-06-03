# M0 Spike — Setup Notes & Findings (SPARC · Completion)

Status: **PASS**. RuVector runs embedded in Rust on Windows and serves k-NN similarity
search locally — no Docker, no Postgres, no network. See `M0-spec.md` for the spec.

## What was built
- `Cargo.toml` — `tovli` binary crate, depends on `ruvector-core` (pure-Rust feature subset).
- `src/vector_store.rs` — `VectorStore` trait + `RuVectorStore` (the seam isolating RuVector).
- `src/main.rs` — ingests 6 sample docs, runs a query, prints results, self-checks vs the spec.

Run it:
```powershell
cargo run --quiet
```
Expected: 5 PASS checks and `RESULT: PASS`. Sample output:
```
  #1  doc-arch-2    dist=0.0017  topic=architecture  ...
  #2  doc-arch-1    dist=0.0063  topic=architecture  ...
  #3  doc-deploy-2  dist=0.8375  topic=deployment    ...
latency: ~250µs
```

## Decisions made during M0 (resolves parts of PRD §16)
- **#1 (engine):** Use the **`ruvector-core` Rust crate** (crates.io v2.2.0), embedded.
  RuVector is natively Rust; the npm package (0.2.27) is just one binding and lags the
  crate. `ruvector-postgres` (pgrx extension) + Docker are **deferred** — "add later".
- **Language:** Rust (not the PRD's original TypeScript). RuVector's primary form is a
  Rust crate, so Rust gets the most direct, most up-to-date access.
- **Embeddings:** none yet. `ruvector-core`'s `HashEmbedding` is a non-semantic placeholder
  (the crate warns: "dog"/"god" look similar). The spike uses hand-crafted deterministic
  vectors so correctness is verifiable. Real embeddings (ONNX/API) are an M1+ decision.

## Toolchain — Windows gotchas (non-obvious, save time later)
1. **No Docker** by design (user deferred). The whole spike avoids it.
2. **Rust toolchain:** installed via rustup, host `x86_64-pc-windows-gnu`, stable 1.96,
   minimal profile (avoids the multi-GB MSVC / VS Build Tools install).
3. **GNU toolchain needs mingw `dlltool`.** A bare `cargo build` of the dependency tree
   fails with `error calling dlltool 'dlltool.exe': program not found`. Fix: install
   **WinLibs (POSIX/MSVCRT)** mingw-w64 and put its `mingw64\bin` on PATH. It ships
   `dlltool.exe` + `gcc.exe` and its MSVCRT runtime matches the gnu toolchain.
   - Installed at:
     `%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.MSVCRT_*\mingw64\bin`
   - This PATH entry is needed for every `cargo build`/`run` until it's added to the
     user PATH permanently.
4. **Feature flags matter.** `ruvector-core`'s default `simd` feature pulls `simsimd`,
   which compiles C and needs a C compiler. We build with:
   `--no-default-features --features "storage,hnsw,parallel"` (pure Rust; scalar distance
   fallback, fine for a spike). `gcc` from WinLibs is now present, so `simd` *could* be
   re-enabled later as an experiment (PRD M9).

## How to add RuVector-Postgres + Docker later (the "add later" path)
The PRD's original M0 used `ruvector-postgres` (a pgrx Postgres extension) via Docker.
To bring it in without disturbing the app:
1. Add a Docker Compose service running Postgres with the `ruvector` extension
   (the `crates/ruvector-postgres` pgrx extension; needs `cargo install cargo-pgrx`).
2. Implement the existing `VectorStore` trait with a Postgres-backed type
   (e.g. `PgVectorStore`) using `sqlx`/`tokio-postgres`.
3. Swap the impl behind a config flag. `main.rs` and the rest of the app don't change —
   that's the point of the `VectorStore` seam (PRD §15 Risk-1).

## Next (per PRD recommended order, Rust-adapted)
- **M1 — Ingestion:** folder scan, Markdown-aware chunking, content hashing, real
  embeddings, `tovli ingest`. Best run as a SPARC cycle (Spec → Pseudocode → Arch →
  Refine → Complete) given its size.
