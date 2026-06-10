# Roadmap

This roadmap describes likely project direction, not a commitment to dates.

## Current Focus

- Keep the CLI workflow stable: `ingest`, `search`, `eval`, and `ask`.
- Improve documentation for open-source contributors.
- Preserve deterministic local development and CI.
- Improve retrieval observability and evaluation coverage.

## Near-Term Work

- Add more parser coverage and parser tests.
- Improve retrieval explain output.
- Add more realistic evaluation datasets.
- Store full chunk content for answer context instead of using retrieval
  previews only.
- Add a real LLM provider adapter behind the existing `LlmPort`.
- Improve answer log querying and inspection.

## Future Work

- Feedback capture and retrieval debugging.
- Feedback collection for retrieved chunks and generated answers.
- HTTP API.
- Bot integrations.
- PDF support.
- Additional source connectors.
- Release packaging and installation instructions.

## Design Constraints

- The default workflow should remain offline and deterministic.
- Retrieval quality should be measurable before generation is expanded.
- Generated answers must remain grounded in retrieved sources.
- Provider integrations should stay behind ports and feature gates where
  practical.
