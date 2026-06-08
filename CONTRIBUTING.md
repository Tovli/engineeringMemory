# Contributing to tovli

Thanks for your interest in contributing to `tovli`. This project is a
local-first Rust CLI and library for learning and building practical vector
retrieval and RAG workflows over engineering documents.

## Good First Contributions

Useful areas for early contributors include:

- Improving parsers for supported file types.
- Adding retrieval evaluation examples.
- Adding focused tests around ingestion, retrieval, evaluation, or answer
  generation behavior.
- Improving CLI help text and documentation.
- Adding provider adapters behind existing ports.
- Improving diagnostics for bad retrieval, weak answers, or index state.

## Development Setup

Install Rust with `rustup`, then verify the pinned toolchain:

```powershell
cargo --version
rustup show
```

Build and run the CLI help:

```powershell
cargo run --bin tovli -- --help
```

Run the local RuVector spike:

```powershell
cargo run --bin tovli -- spike
```

## Test Commands

Run tests serially:

```powershell
cargo test -- --test-threads=1
```

Run Clippy:

```powershell
cargo clippy --all-targets
```

The serial test run is intentional because integration tests open multiple
embedded vector stores.

## Design Principles

Please keep contributions aligned with these project constraints:

- Keep the default workflow offline and deterministic.
- Keep CLI handlers thin; put behavior in library modules.
- Preserve domain and application logic behind ports instead of coupling it to
  infrastructure crates.
- Prefer focused tests over broad, brittle assertions.
- Make retrieval behavior observable when changing ranking, filtering, scoring,
  or answer generation.
- Document architecture changes with an ADR under `docs/adrs/` when they affect
  module boundaries, public behavior, storage, scoring, or provider contracts.

## Pull Request Checklist

Before opening a pull request:

1. Run `cargo test -- --test-threads=1`.
2. Run `cargo clippy --all-targets`.
3. Update `README.md`, `ROADMAP.md`, or the relevant docs when behavior changes.
4. Add an ADR for significant architecture decisions.
5. Keep unrelated refactors out of the PR.

## Commit Style

Use concise, descriptive commit messages. Examples:

```text
add yaml parser tests
improve retrieval explain output
document answer citation policy
```

## Reporting Issues

When reporting a bug, include:

- The command you ran.
- The expected behavior.
- The actual behavior.
- Relevant logs or output.
- Your OS and Rust version.
- Whether you used the default ONNX embedder, `--mock`, or `--no-default-features`.

Do not include private documents, API keys, secrets, or proprietary source text
in public issues.
