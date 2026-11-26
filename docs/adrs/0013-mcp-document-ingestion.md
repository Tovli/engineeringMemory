# ADR-0013: Expose document ingestion to AI clients through a local MCP adapter

- **Status:** Proposed
- **Date:** 2026-11-12
- **Milestone:** AI-client ingestion (scope extension; no existing milestone renumbering)
- **Context refs:** [Ingestion context](../ddd/contexts/ingestion.md), ADR-0001, ADR-0002,
  ADR-0009, [ADR-0012](0012-local-api-thin-adapter-over-services.md)

## Related decisions

This ADR owns the MCP interface. Application guarantees and image support are separate
decisions so they can be reviewed and delivered independently.

| ADR | Owns | Delivery dependency |
|-----|------|---------------------|
| [0014](0014-durable-single-document-ingestion.md) | Single-document ingestion, identity, retries, recovery, and temporary store locking | Required for text ingestion over MCP |
| [0015](0015-image-memory-assets-and-search.md) | Original image preservation and searchable text representations | Required for the image tool extension |
| [0016](0016-text-image-attachments.md) | Links between text and image memories, including captions and chunk anchors | Required for attachment tools |

## Context

Users should be able to tell an MCP-capable AI client to save an engineering note in tovli's
embedded RuVector index. Today `tovli ingest <folder>` accepts local files; there is no MCP
server or application entry point for ingesting inline content.

The existing ingestion path is not just a vector insert. `IngestionOrchestrator` parses,
chunks, embeds, and writes document records, RuVector vectors, a document-to-chunk sidecar,
and the full-text keyword index. Retrieval joins candidates with document metadata and
filters deleted documents. Inserting vectors alone would bypass those contracts.

The orchestrator also scans a folder and deletes documents missing from that scan. Calling
it on a temporary folder for every tool call would couple a single-document addition to
folder reconciliation. Its unchanged-content shortcut does not cover a request's metadata,
and the stores do not share one transaction. These are application concerns to resolve
before exposing writes to an AI client.

ADR-0012 proposes HTTP handlers over existing services, but that API is not implemented.
MCP ingestion must work independently of it. Image memories also need preserved bytes,
searchable descriptions, and explicit relationships with text; a text-only tool cannot
preserve an image by storing its filename or description alone.

## Decision

Add a **thin inbound MCP adapter over the Ingestion application layer**, shipped as an
optional `tovli-mcp` binary in this crate. An AI client sends document text and metadata;
tovli generates embeddings locally and persists all indexes used by its existing retrieval.

### Transport and runtime

- Start with **stdio**, launched by a local MCP client. Clients that require a remote HTTP
  endpoint are outside this first version. There is no listening network port.
- Use the official Rust SDK, `rmcp`, with Tokio and its stdio transport. Delegate protocol
  initialization, version negotiation, tool discovery, and dispatch to the SDK.
- Gate MCP dependencies and the binary behind a non-default `mcp` Cargo feature. Select and
  pin compatible dependency versions during implementation. Check against the declared
  Rust 1.77 minimum; a required MSRV increase must be explicit in Cargo metadata and docs.
- Run blocking operations on one dedicated worker with a bounded request channel. Keep the
  embedder on that worker and open store handles under the lock for each operation. Current
  ports do not promise `Send + Sync`.
  Async MCP handlers await results without blocking Tokio executor threads.
- Reserve stdout for protocol messages. Send startup, embedding, and diagnostic logs to
  stderr without raw document content. Do not invoke CLI handlers that print to stdout.

### Text tool contract

Start with two tools with JSON Schema inputs and outputs. Tool names are stable public API.
Reject unknown fields and validate limits before parsing, embedding, or writing.

| Tool | Input | Behavior and result |
|------|-------|---------------------|
| `tovli_add_document` | Required `source_key`, `content`. Optional `format` (`markdown` default or `text`), `title`, `project`, `tags`, and `metadata` (JSON object). | Add one document. Return `status` (`created` or `unchanged`), `document_id`, `source_uri`, `content_hash`, `chunk_count`, `embedding_model`, `embedding_dimension`, `indexed_at`. |
| `tovli_query` | Required `source_key` in the first release | Look up an active, committed memory by exact key. Return a `results` array with one memory on a hit or zero on a miss. |

Initial limits: `source_key` matches `[A-Za-z0-9][A-Za-z0-9_-]{0,127}`; content is nonblank
UTF-8 text, at most 256 KiB; title at most 256 characters; project at most 128 characters;
at most 32 nonblank tags of at most 64 characters each. The text-only delivery bounds each
protocol message to 1 MiB before buffering it in full and the ingestion queue to 16 requests;
return `busy` when the queue is full. The image extension changes framing limits below.
Treat these as documented server limits, not client suggestions.

Use `metadata` for application fields such as an incident ID, a review flag, or an
external reference. It defaults to `{}` and is limited to 4 KiB of compact UTF-8 JSON.
The size limit bounds the cost of copying it to each RuVector chunk. The dedicated
`title`, `project`, and `tags` fields remain available for their existing uses.

This tool call saves a text memory.

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "tovli_add_document",
    "arguments": {
      "source_key": "payment-timeout-investigation-2026-11-12",
      "content": "# Payment timeout investigation\nThe worker retries after 30 seconds.",
      "format": "markdown",
      "project": "payments",
      "tags": ["incident", "timeouts"],
      "metadata": {
        "incident_id": "INC-431",
        "severity": 2,
        "reviewed": true
      }
    }
  }
}
```

Call `tovli_query` with `{"source_key":"payment-timeout-investigation-2026-11-12"}` to
retrieve the saved memory. A result includes `source_key`, `source_kind`, the committed
receipt fields (`document_id`, `source_uri`, `content_hash`, `chunk_count`,
`embedding_model`, `embedding_dimension`, `indexed_at`), title, project, tags,
`metadata`, and a bounded text preview. An empty `results` array means there is no active, committed
memory for that key. An empty argument object fails input validation. Future search
selectors can make `source_key` optional while preserving exact lookup and the array
response. Ranked search and custom metadata filters are deferred.

Return the receipt or query result in `structuredContent` and the same JSON serialized
in a text content block for client compatibility. Publish an `outputSchema` covering
success and error results. Business/storage failures use `isError: true` with stable codes such as
`source_conflict`, `embedding_model_mismatch`, `busy`, `store_in_use`, `storage_error`,
or `recovery_required`. Malformed protocol calls, unknown tools, and schema-invalid arguments
use SDK protocol errors.
Errors identify whether retrying the same request is appropriate and never claim indexing
succeeded after a partial write.

Annotate addition with `readOnlyHint: false`, `destructiveHint: false`,
`idempotentHint: true`, and `openWorldHint: false`. Annotate the query with
`readOnlyHint: true`. The application enforces validation and create-only semantics.
There is no server-side LLM call, sampling, URL fetching, file browsing, shell execution,
client-supplied vector, overwrite, delete, or bulk ingestion in this version.
Image input and retrieval are explicit additions below; the text tool never interprets
Markdown image references as permission to read files or fetch URLs.

### Application boundary, identity, and retries

Delegate creation and exact lookup to the application operations defined in
[ADR-0014](0014-durable-single-document-ingestion.md). It owns source identity,
metadata-aware fingerprints, model guards, receipts, and recovery.
The adapter maps fields and errors; it must not insert vectors directly, create temporary
folders for ingestion, or implement a second retry policy.

### Shared store access

Require an explicit absolute `--store-dir` startup option. Point it at the `.tovli`
directory in the working directory used for CLI commands to share that store.
The CLI can access it while the MCP server stays connected. For each tool call that accesses the
store, the worker acquires the lock from ADR-0014, checks recovery, and closes the
handles before releasing the lock. If the CLI holds the lock, the worker waits up to
five seconds and returns retryable `store_in_use` on timeout. The CLI follows the same
policy when an MCP call holds the lock. An idle server leaves the store available.

After losing a response, reconnect and query by key to confirm that the memory exists.
To verify that an ambiguous save used the same content and metadata, retry the identical
request. It returns the original receipt for
a match or `source_conflict` for a different payload. Pending and failed journal entries
are internal to recovery. A blocked store returns `recovery_required` to the query.

### Image and attachment tools

The current worktree includes both text and image ingestion. Implement and verify them
as separate tasks when useful. The image and attachment tools depend on ADR-0015 and
ADR-0016. During development, advertise tools as their backing operations become
available. Release packaging and timing remain open.

| Tool | Input | Behavior and result |
|------|-------|---------------------|
| `tovli_add_image` | Required `source_key`, `image_base64`, `mime_type`, `description`; optional `ocr_text`, `context`, `field_provenance`, `title`, `project`, `tags` | Create an image memory through ADR-0015. Return the text ingestion receipt plus `asset_id`, original-byte hash, verified MIME type, and byte size. No implicit attachment. |
| `tovli_get_image` | Required `asset_id` | Fetch the preserved original as MCP image content, with verified MIME type and an identifying text block. Return `asset_not_found` or `asset_missing` when appropriate. |
| `tovli_link_image` | Required text `document_id`, `image_document_id`; optional `chunk_id`, `caption` | Create the relationship defined by ADR-0016. Return `link_id`, both document IDs, resolved `asset_id`, anchor state, and `created` or `unchanged`. |
| `tovli_list_image_links` | Exactly one of `document_id`, `image_document_id`; optional target `project`/`tags` filters, opaque `cursor`, `limit` (default 20, maximum 100) | Return active relationships and `next_cursor`, supporting text-to-images and image-to-text lookup. No image bytes are included. |

For the image extension, raise the framing limit to 16 MiB for both incoming and outgoing
messages while retaining the 1 MiB per-request limit for text tools. Limit decoded originals
to 8 MiB, at most 40 million pixels, and derived description/OCR/context together to 256 KiB.
Validate declared and actual MIME type and dimensions before commit. Start with PNG, JPEG,
and WebP; larger images and other formats fail explicitly rather than being silently resized.
Apply byte-based admission as well as the 16-request count limit: queued decoded image data
must not exceed 32 MiB. Client attachments are not automatically forwarded to MCP; the client
must supply the bytes explicitly. Chunked uploads and remote fetching are deferred.

Once image support is enabled, `tovli_query` also resolves image memory keys and returns
the image receipt fields, including `asset_id`. Fetch the original with `tovli_get_image`.

A client can handle “save this note with this image” as one user request. It calls
`tovli_add_document` and `tovli_add_image` as needed, then calls `tovli_link_image`
with the committed document IDs. It can reuse an existing image memory and retry the
link independently if that step fails.

Use the same create-only annotations for image addition and linking. Reads are read-only;
image retrieval uses MCP image content rather than a JSON-only receipt schema. Link IDs and
asset IDs are data, never host paths. A client can fetch a selected image for visual inspection
after retrieving text or listing its attachments; this extension adds no server-side vision
or OCR model. Existing text ingestion fingerprints do not change when a link is added.
`field_provenance` maps description/OCR/context fields to their origin and optional generator
information as defined by ADR-0015. Link cursors are bound to the endpoint and requested
filters; a cursor from another query is invalid. Image/list/link errors follow the same
sanitized code contract, including `link_conflict`, `invalid_anchor`, and `asset_missing`.

### Client configuration and delivery

The following is a **proposed configuration**, usable only after the binary is implemented
and built with `cargo build --release --features mcp --bin tovli-mcp`. It illustrates a
common client configuration shape; the enclosing settings location depends on the client.

```json
{
  "mcpServers": {
    "tovli": {
      "command": "C:/path/to/tovli-mcp.exe",
      "args": ["--store-dir", "C:/path/to/workspace/.tovli"]
    }
  }
}
```

The store location is operator configuration, never a tool argument or client working-directory
assumption. Model files should be provisioned locally for offline use; retain the existing
documented ONNX first-use download behavior. `--mock` is for a separate test store.
The local OS account and client process form the access boundary; this design adds no remote
authentication. Staged input and stored notes receive the same local-data protection as the
existing index. Future HTTP transport needs its own authentication and exposure decision.

Implementation should deliver the feature-gated adapter, client setup instructions, and
focused integration tests over the services and guarantees specified in ADRs 0014-0016.
The local HTTP adapter in ADR-0012 has its own delivery scope and uses the same store
policy. A future HTTP ingestion handler can call the shared application operation directly.

## Alternatives considered

| Option | Reason not selected |
|--------|---------------------|
| Expose raw RuVector insert tools or Ruflo `memory_store` | They do not enforce tovli's document, chunk-map, embedding, and keyword-index contracts; Ruflo's own memory is a separate store. |
| MCP wrapper invoking `tovli ingest` on temporary folders | Inherits folder deletion semantics, process startup/locking costs, and console output instead of a typed single-document operation. |
| MCP proxy over the proposed HTTP API | Depends on an unimplemented API that does not yet propose an ingestion endpoint. |
| Streamable HTTP from the first release | Useful for remote-only clients and shared ownership, but adds a network access/authentication design beyond local addition. |
| Upsert arbitrary existing sources | Adds destructive replacement, revision, and conflict semantics; create-only addition meets the initial need with predictable retries. |

## Consequences

- **+** An AI client can save text directly into the same local memory searched by tovli,
  without choosing an embedding provider or manipulating database files.
- **+** CLI and MCP share parsing, chunking, embedding, and indexing behavior; retrieval
  keeps its existing filtering, scoring, and hybrid-search contracts.
- **+** Exact lookup confirms a saved memory. Durable receipts make ambiguous responses
  safe to retry.
- **+** The CLI and MCP can use the same store while the MCP session stays open.
- **-** Temporary store locks serialize operations. A long CLI command can delay an MCP call.
- **-** Create-only notes require a new source key for revisions.
- **-** Inline notes become persistent local data. The client decides what to send, and
  ordinary stored text must never be executed or treated as server instructions.

## Acceptance criteria for implementation

1. A real stdio session initializes, lists the tools, accepts the example addition, and
   returns a schema-valid receipt. Querying its key returns the same document and committed
   receipt fields. An unknown key returns an empty array and an empty argument object
   fails validation. Stdout contains protocol messages only.
2. While the server remains connected, CLI vector, keyword, and hybrid search can
   retrieve the added note with its source URI, project, and tags using the same model.
   CLI ingestion and subsequent MCP calls work in the same session. Overlapping
   commands wait for the lock or time out with retryable `store_in_use`.
3. Identical retries, including after restart or a lost response, keep document/chunk
   counts stable and return the original receipt. Different metadata or content with the
   same key returns a conflict without modifying committed data. Exact lookup returns
   custom metadata with JSON types preserved. Oversized metadata fails before indexing.
4. A filesystem ingestion run leaves MCP documents intact. Existing CLI update, deletion,
   force, dry-run, retrieval, and evaluation behavior remains covered by regression tests.
5. Failure injection between each persistence step, including vector/sidecar writes,
   proves restart repair and retry produce one complete document without orphan chunks,
   duplicate mappings, or premature success. If recovery is blocked, both addition and
   query return `recovery_required`.
6. Model mismatch, empty/oversized input, unknown fields, queue saturation, competing
   processes, and cancellation produce documented results without corrupting the index.
7. Deterministic tests use a temporary store and mock embeddings without network access.
   Validate default and `--no-default-features --features mcp` builds, the supported Rust
   version, serial tests, and Clippy; manually smoke-test one stdio-capable AI client.
8. In the completed worktree, a client can add text, add an image, look up the image
   by its key, link the document IDs, list the link in either direction, and fetch
   byte-identical image content. A failed link can be retried using the committed IDs.
   Invalid MIME types, image/message/queue limits, and dangling link endpoints fail explicitly.

## References

- [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [MCP tools, structured results, and errors](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

Protocol references identify a published baseline, not a hard-coded negotiated version.
Implementation must test the protocol versions supported by the pinned SDK and target client.
