# ADR-0012: The local API is a thin async adapter over the existing application services

- **Status:** Proposed
- **Date:** 2026-06-29
- **Milestone:** M7 - Local API
- **Context refs:** PRD section 13 Milestone 7, `docs/ddd/context-map.md`, ADR-0001, ADR-0002, ADR-0003, ADR-0006, ADR-0008, ADR-0009, ADR-0011

## Context
Milestone 7 exposes tovli over a local HTTP API: search, ask, feedback, and document-listing endpoints,
plus an OpenAPI document and runnable examples. The PRD acceptance criteria are explicit and constraining:
API calls must use the *same services* as the CLI, responses must be structured JSON, the API must support
search/ask/feedback, and the existing CLI must keep working. The stated learning outcome is to separate
retrieval services from interface layers.

The current architecture already makes this feasible without restructuring the core:

1. Retrieval is a read-only hexagonal module that owns the search contract and emits an immutable
   `RetrievalRun` (ADR-0001), with filtering and score semantics fixed in the application layer
   (ADR-0002, ADR-0003, ADR-0009).
2. RAG sits behind an `LlmPort` seam and the domain enforces the citation protocol and no-answer /
   exit-code policy (ADR-0006, ADR-0008).
3. Feedback is an append-only observability log exposed through application services
   (`FeedbackService`, `FeedbackReportService`) that depend on repository ports, not concrete storage;
   ADR-0011 already anticipated that "a later API milestone may add HTTP handlers over the same
   application services" and that "the same Feedback services can later be reused by the local API."
4. `main.rs` is already only CLI wiring: it parses arguments, opens redb / RuVector stores by path, and
   calls application services. The transport layer holds no domain logic today.

So the M7 question is not *how to build retrieval logic* — it already exists — but *how to add a second
inbound adapter (HTTP) beside the CLI without duplicating or leaking core logic, and which runtime/stack
is the right state-of-the-art choice for a local-first Rust binary that is otherwise fully synchronous.*

The relevant tension: the core is synchronous (redb, `ruvector-core`, and the `ureq` HTTP client are all
blocking; there is no async runtime in the dependency tree today). A modern Rust HTTP server stack (Axum on
Tokio) is async. Introducing it means deciding how synchronous service calls run under an async server, and
how the API process coexists with the CLI over the same on-disk stores.

## Decision
Build the M7 API as a **thin inbound HTTP adapter** that calls the existing application services directly —
the same services the CLI calls — and add no retrieval, ranking, RAG, or feedback logic in the transport
layer.

- Ship the API as a separate binary (e.g. `tovli-api`) in the same crate, reusing the library's application
  services and infrastructure adapters. The default `tovli` CLI binary is unchanged.
- Use **Axum on Tokio** as the HTTP stack (the current state-of-the-art async Rust web framework). Because
  the core services are synchronous and CPU/IO-blocking (redb, RuVector, ONNX embedding, blocking LLM HTTP),
  handlers invoke services via `tokio::task::spawn_blocking` rather than calling blocking code on async
  worker threads. The async/sync boundary lives entirely in the adapter.
- Endpoints map one-to-one onto existing service calls and reuse the existing serde DTOs as the wire format:
  - `GET  /search`        → retrieval/search service → `RetrievalRun` JSON
  - `POST /ask`           → RAG service → answer + citations JSON
  - `POST /feedback`      → `FeedbackService::record(_many)`
  - `GET  /feedback/report` → `FeedbackReportService::generate`
  - `GET  /documents`     → document listing
  - `GET  /healthz`       → liveness
- Search and ask responses include the `query_id` and `retrieval_run_id` so a subsequent `POST /feedback`
  can be validated against persisted retrieval-run evidence, exactly as the CLI flow does. The API does not
  invent a separate feedback path; it reuses the run-evidence store and the displayed-chunk validation from
  ADR-0011.
- The domain's no-answer / exit-code policy (ADR-0008) maps to HTTP status semantics: a successful answer is
  `200`, a deliberate no-answer is a structured `200`/`204`-style body (not a `5xx`), client errors
  (unknown run id, chunk not displayed, malformed request) are `4xx`, and only genuine internal failures are
  `5xx`. Citation tags and scores cross the wire unchanged so callers see the same evidence the CLI prints.
- Generate the **OpenAPI document from the code** (e.g. `utoipa` derive on handlers and DTOs) and serve it at
  a stable path, rather than hand-maintaining a spec that can drift from the handlers. The committed spec and
  request/response examples are the M7 deliverables.
- The server **binds to loopback (`127.0.0.1`) by default** and ships with **no authentication** in M7,
  consistent with the local-first, single-tenant storage style. It is not a remotely exposed service.
- The API and CLI share store *paths* but follow redb's single-process / single-writer model: the API is the
  long-running owner of the stores while it runs. Concurrent writes from a separate CLI process against the
  same database files are out of scope for M7.
- Any future remote exposure, multi-tenant access, authentication/authorization, or rate limiting must be a
  separate ADR; it is explicitly not introduced here.

## Consequences
- **+** The PRD acceptance criteria are met structurally: handlers call the same services as the CLI, so
  search/ask/feedback behaviour cannot diverge between interfaces, and the CLI is untouched.
- **+** The hexagonal seam is demonstrated end to end — two inbound adapters (CLI, HTTP) over one core — which
  is the milestone's learning outcome.
- **+** Reusing existing serde DTOs as the wire format keeps the API honest about scores, ranks, search mode,
  and citations, and means the OpenAPI schema is derived from the same types the domain already serializes.
- **+** Feedback over HTTP inherits ADR-0011's append-only guarantees and displayed-chunk validation for free,
  because it goes through the same `FeedbackService` and run-evidence store.
- **+** The same adapter pattern and DTOs are directly reusable by the M8 Telegram bot, which the PRD requires
  to contain no core retrieval logic.
- **-** Introducing Axum/Tokio pulls an async runtime into a previously synchronous binary. The async/sync
  boundary (`spawn_blocking`) must be applied consistently or blocking service calls will starve the async
  executor. This complexity is new surface area to test.
- **-** Code-generated OpenAPI couples handlers to annotation macros; the spec is only as correct as the
  annotations, so example-driven contract tests are needed to keep it trustworthy.
- **-** Sharing redb store paths with the CLI means the API effectively owns the stores while running;
  documentation must warn against concurrent CLI writes to the same database to avoid lock contention or
  corruption.
- **-** Loopback-only and no-auth are correct for local-first but mean the API is not deployable as-is; a
  later milestone needs an explicit security ADR before any remote use.
- **Future:** Authentication, remote binding, multi-tenant isolation, streaming responses (SSE/WebSocket for
  `ask`), and rate limiting can supersede the local-only posture only through a new ADR that defines the
  threat model, auth mechanism, and backward-compatibility for existing local callers.
