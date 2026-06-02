# tovli

A local-first technical knowledge assistant built on **RuVector** — ingest your engineering
docs, retrieve relevant chunks by similarity, and (later) generate cited answers. See
[`docs/prd.md`](docs/prd.md) for the full product spec and [`docs/ddd/`](docs/ddd) for the domain model.

> **Stack:** Rust + the embedded [`ruvector-core`](https://crates.io/crates/ruvector-core)
> crate. No Docker required. (RuVector-Postgres + Docker are a planned "add later" option.)

## Status

| Milestone | What | State |
|-----------|------|-------|
| **M0** | RuVector technical spike — embed RuVector in Rust, insert vectors, k-NN search | ✅ **done** |
| M1 | Document ingestion (`tovli ingest`) | next |
| M2+ | Search CLI, evaluation, RAG, hybrid search, feedback, API, bot | planned |

## Quick start (M0 spike)

Prerequisites (one-time, Windows):
- Rust (`x86_64-pc-windows-gnu`): `rustup` with stable toolchain.
- mingw-w64 for the GNU linker's `dlltool`: `winget install BrechtSanders.WinLibs.POSIX.MSVCRT`,
  then add its `mingw64\bin` to PATH. (Details + why in [`docs/spike/M0-setup.md`](docs/spike/M0-setup.md).)

Run the spike:
```powershell
cargo run --quiet
```
It ingests 6 sample documents, runs a similarity query, and self-checks the nearest
neighbours against [`docs/spike/M0-spec.md`](docs/spike/M0-spec.md). Expect `RESULT: PASS`.

## Layout
```
src/
  main.rs           # thin entrypoint: sample data, query, self-checks (no engine logic)
  vector_store.rs   # VectorStore trait + RuVectorStore — the seam isolating RuVector
docs/
  prd.md            # product requirements
  ddd/              # domain-driven design model
  spike/            # M0 spec + setup notes
```
