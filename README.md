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
| **M1** | Document ingestion (`tovli ingest`) — scan, markdown-aware chunk, embed, store, idempotent | ✅ **done** |
| **M2** | Retrieval CLI (`tovli search`) — vector search, project/tag/source filters, `--explain` | ✅ **done** |
| **M3** | Retrieval evaluation (`tovli eval`) — Hit@1/3/5 + MRR, JSON report, CI threshold gate | ✅ **done** |
| **M4** | RAG answer generation (`tovli ask`) — cited answers, no-answer policy, prompt versioning | ✅ **done** |
| M5+ | Hybrid search, feedback, API, bot | planned |

## Setup

Prerequisites (one-time, Windows):
- Rust via `rustup`. The repo pins `stable-x86_64-pc-windows-msvc` in
  [`rust-toolchain.toml`](rust-toolchain.toml), so `cargo` will automatically use that toolchain.
- Visual Studio C++ build tools / MSVC and a Windows SDK. The mock/offline path is light, but the
  ONNX feature depends on Windows MSVC binaries.

Install Rust:
```powershell
winget install --id Rustlang.Rustup -e --source winget
```

If the current PowerShell session still cannot find `cargo`, either open a new terminal or refresh
PATH for this session:
```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo --version
rustup show
```

If MSVC is missing, open the Visual Studio Installer and add **Desktop development with C++**
including the Windows SDK, or install Visual Studio Build Tools with the same workload.

The first Cargo run downloads dependencies from crates.io. No Docker, Postgres, ONNX model, or
mingw-w64 install is required for the default mock-embedder commands. The older GNU/WinLibs notes
are kept as historical M0 spike context in [`docs/spike/M0-setup.md`](docs/spike/M0-setup.md).

## Quick start

Run the spike:
```powershell
cargo run --bin tovli -- spike
```
It ingests 6 sample documents, runs a similarity query, and self-checks the nearest
neighbours against [`docs/spike/M0-spec.md`](docs/spike/M0-spec.md). Expect `RESULT: PASS`.

## Ingest documents (M1)

```powershell
# Mock embedder (fast, offline, no model needed) — light default build:
cargo run --bin tovli -- ingest ./docs

# Real local ONNX/MiniLM embeddings (requires models/ cached — see docs/sparc):
cargo run --features onnx --bin tovli -- ingest ./docs

# Options: --dry-run  --force  --project <name>  --tag <t>  --mock
```
Scans the folder, chunks markdown by heading (never splitting code fences), embeds each
chunk, and stores vectors in RuVector with a redb document sidecar. Re-running skips
unchanged files (content-hash idempotency). State lives in `.tovli/` (gitignored).
See [`docs/sparc/m1-document-ingestion/`](docs/sparc/m1-document-ingestion) for the SPARC
spec → pseudocode → architecture → completion trail.

## Ask a question (M4)

```powershell
# Cited answer generated from retrieved chunks (mock LLM, offline by default):
cargo run --bin tovli -- ask "What do our docs say about score semantics?" --mock

# Options: --top-k <n>  --show-context  --no-llm (retrieval only)  --mock
```
Retrieves the most similar chunks, sends only those to the LLM (behind an `LlmPort` seam), and prints
a concise answer with a mandatory `Sources:` list. Weak retrieval (nothing above the similarity
threshold) yields an explicit *no-answer* response instead of a hallucination; citations are validated
against the retrieval run, so the model can't invent sources. Each answer is logged to
`.tovli/answers.jsonl` with its prompt-template version. The default build uses a deterministic
`MockLlm` (no network); a real provider is a future feature-gated adapter behind the same seam.
See [`docs/sparc/m4-rag-answer-generation/`](docs/sparc/m4-rag-answer-generation) and
[ADRs 0006–0008](docs/adrs).

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
