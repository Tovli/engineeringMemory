# tovli

`tovli` is a local-first technical memory assistant for engineering documents.
It ingests a folder of notes, chunks the files, embeds the chunks, stores them
in an embedded RuVector index, and lets you search or ask questions from the
command line.

The project is designed as a practical, open-source learning implementation of
a retrieval-augmented generation pipeline. The default embedding path is local
ONNX/MiniLM, so no Docker, Postgres, or cloud API key is needed. A deterministic
mock embedder remains available with `--mock` or `--no-default-features` for
lightweight development and CI.

## Features

- Recursive local document ingestion.
- UTF-8 parsers for `.md`, `.txt`, `.json`, `.yaml`, and `.yml`.
- Markdown-aware chunking with heading paths and code-fence preservation.
- Embedded vector storage with `ruvector-core` and `redb`.
- Local ONNX/MiniLM embeddings by default.
- Deterministic mock embeddings for repeatable local development and CI.
- Vector search with `--top-k`, `--project`, `--tag`, `--source`, and
  `--explain`.
- Retrieval evaluation with Hit@1, Hit@3, Hit@5, MRR, latency, and an optional
  CI threshold gate.
- RAG-style `ask` command with cited answers, citation validation, a no-answer
  policy, and JSONL answer logging.

## Status

The current repository is an early CLI and library implementation. The core
local workflow is working:

- `tovli ingest`
- `tovli search`
- `tovli eval`
- `tovli ask`

Planned work includes hybrid search, feedback collection, a real LLM provider
adapter, full-content answer context, an HTTP API, bot integrations, PDF
support, and additional source connectors.

## Project Docs

- [Contributing](CONTRIBUTING.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md)
- [Support](SUPPORT.md)
- [Roadmap](ROADMAP.md)
- [Changelog](CHANGELOG.md)
- [License](LICENSE.md)

## Requirements

- Rust 1.77 or newer.
- The repository-pinned Rust toolchain in `rust-toolchain.toml`.
- On Windows, Visual Studio C++ Build Tools with the Windows SDK.

Install Rust with `rustup`:

```powershell
winget install --id Rustlang.Rustup -e --source winget
```

If `cargo` is not available in the current PowerShell session, open a new
terminal or refresh `PATH`:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo --version
rustup show
```

## Quick Start

Run the local RuVector smoke test:

```powershell
cargo run --bin tovli -- spike
```

Ingest the project docs with the default local ONNX/MiniLM embedder:

```powershell
cargo run --bin tovli -- ingest ./docs
```

Search the indexed chunks:

```powershell
cargo run --bin tovli -- search "What do the ADRs say about score semantics?" --top-k 5 --explain
```

Ask for a cited answer:

```powershell
cargo run --bin tovli -- ask "What is the citation policy?" --show-context
```

Evaluate retrieval quality:

```powershell
cargo run --bin tovli -- eval ./eval/questions.json --fail-below-hit-at-3 0.6
```

Generated local state is written to `.tovli/`, which is ignored by Git. Delete
that directory to reset the local index and answer log.

## CLI Reference

```text
tovli spike
tovli ingest <folder> [--dry-run] [--force] [--project <name>] [--tag <tag>] [--mock]
tovli search <query> [--top-k <n>] [--mode vector] [--project <name>] [--tag <tag>] [--source <path>] [--explain] [--mock]
tovli eval <questions.json> [--top-k <n>] [--mode vector] [--fail-below-hit-at-3 <fraction>] [--output <path>] [--mock]
tovli ask <query> [--top-k <n>] [--mode vector] [--show-context] [--no-llm] [--mock]
```

Only `--mode vector` is currently implemented. Keyword and hybrid modes are
planned.

## Embedding Providers

The default build enables the `onnx` Cargo feature and uses `OnnxEmbedder`.
On first use, tovli looks for MiniLM in this order:

- `TOVLI_MINILM_DIR`, when set, containing `model.onnx` and `tokenizer.json`.
- `models/all-MiniLM-L6-v2/` in the current working directory.
- The OS user cache, where tovli downloads the files from Hugging Face if they
  are not already present.

The downloaded files are:

- `onnx/model.onnx`
- `tokenizer.json`

Verify the local model files and ONNX runtime stack with:

```powershell
cargo run --bin verify-onnx
```

Pass `--mock` to `ingest`, `search`, `eval`, or `ask` to force the deterministic
mock embedder. To avoid compiling ONNX dependencies entirely, build with
`--no-default-features`.

## Evaluation Dataset

The sample evaluation dataset lives at `eval/questions.json`. Each question
declares the expected source files, and the evaluator reports:

- Hit@1
- Hit@3
- Hit@5
- MRR
- average latency
- empty-result count
- below-threshold count

Use `--fail-below-hit-at-3` in CI to make retrieval regressions fail the run.

## Architecture

The codebase is organized around bounded contexts and hexagonal seams:

```text
src/
  ingestion/           parse, chunk, embed, and store documents
  retrieval/           query embedding, vector search, filtering, ranking
  evaluation/          retrieval quality datasets, metrics, and reports
  answer_generation/   context assembly, prompt rendering, citations, answers
  vector_store.rs      small RuVector spike/store abstraction
```

The CLI in `src/main.rs` is intentionally thin. Domain and application logic
live under the context modules, while infrastructure adapters isolate RuVector,
`redb`, parsers, embedders, report writing, and the mock LLM.

For design background, see:

- [Product requirements](docs/prd.md)
- [Domain model](docs/ddd/README.md)
- [Architecture decision records](docs/adrs/README.md)
- [Milestone implementation notes](docs/sparc/)

## Development

Run the test suite serially:

```powershell
cargo test -- --test-threads=1
```

Run Clippy:

```powershell
cargo clippy --all-targets
```

The serial test run is the documented path because the integration tests open
multiple embedded vector stores.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md)
for setup, test commands, design principles, and the pull request checklist.
Good starter areas include parser coverage, retrieval evaluation examples,
provider adapters, documentation, and small CLI usability improvements.

Before opening a PR:

1. Keep the default workflow offline and deterministic.
2. Add or update focused tests for behavior changes.
3. Run `cargo test -- --test-threads=1`.
4. Run `cargo clippy --all-targets`.
5. Update the relevant docs or ADRs when changing behavior or architecture.

## License

This project is licensed under the [MIT License](LICENSE.md).
