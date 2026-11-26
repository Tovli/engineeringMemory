# ADR-0014: Make single-document creation durable and safe to retry

- **Status:** Proposed
- **Date:** 2026-11-12
- **Milestone:** AI-client ingestion
- **Context refs:** [ADR-0013](0013-mcp-document-ingestion.md),
  `src/ingestion/orchestrator.rs`, `src/ingestion/ports.rs`,
  `src/ingestion/infra/ruvector_store.rs`, `src/ingestion/infra/redb_repo.rs`

## Context

The folder orchestrator combines document processing with filesystem scanning and deletion
reconciliation. MCP needs a single-document entry point without missing-file deletion.
Its clients can retry after losing a response, including after a process restart.

The current content-hash shortcut does not account for metadata changes. Persistence spans
documents, vectors, the chunk-map sidecar, and keyword records without one transaction.
Vector insertion can succeed before the sidecar commits. The sidecar alone cannot identify
all partially written chunks. CLI commands and an MCP server must be able to run together
while coordinating access to the embedded stores.

These guarantees belong to the application and persistence layers, independently of MCP.

## Decision

Provide a reusable single-document application service with a durable create-only operation.
Use a temporary exclusive lock for each store operation. The CLI and MCP server can stay
running against the same store and take turns accessing its files.
[ADR-0013](0013-mcp-document-ingestion.md) maps the application operation to MCP.

### Shared document processing

Extract parsing, chunking, embedding, and index persistence from the folder orchestrator.
Reuse `FileParser`, `ChunkingService`, `EmbeddingService`, `VectorStorePort`,
`KeywordIndexPort`, and `DocumentRepository`. The folder orchestrator keeps scanning,
updates, force/dry-run options, and missing-file reconciliation; preserve its existing
public behavior. The new create-only contract applies to client-created logical sources.
The supported CLI entry point remains `tovli ingest <folder>`. A standalone file argument
has no separate CLI contract in this decision. An MCP client that wants to save a local
file reads its text and passes that text to `tovli_add_document`.

Markdown input maps to `md`, plain text to `txt`. Propagate an explicit title over a parsed
title and into chunk metadata; keep project and tags on the document and full chunk text
in the keyword index. Model-generated content is ordinary input, never an instruction to
execute commands or resolve external sources.

### Custom document metadata

Persist the caller's `metadata` object on the document record as the source of truth for
exact lookup. Records without that field read as `{}`. During indexing, copy a nonempty
object to each chunk's RuVector `VectorEntry.metadata` under the dedicated
`user_metadata` key, preserving JSON types. Keep the existing system fields at the
top level, so a caller's `document_id` key inside the object cannot replace the chunk's
own document ID. The document lookup returns the canonical object directly. This works
with typed JSON even though the existing vector search adapter projects string metadata
for its output. Custom fields do not enter embedded text or keyword matching.

The bounded copy lets a RuVector record carry user fields but adds storage per chunk.
[ADR-0013](0013-mcp-document-ingestion.md) defines the MCP size limit. Custom filtering
would use the document metadata contract and needs a separate retrieval decision.

### Identity and idempotency

- Use `mcp://local/{source_key}` as the canonical logical source and store it in
  `source_path`. Derive IDs using existing `document_id(...)` and `chunk_id(...)` functions.
  Project is metadata, not identity. Display file name/extension derives from key and format.
- Persist a versioned fingerprint of exact content bytes, normalized format/title/project,
  sorted unique tags, custom metadata, source kind, and any kind-specific fields. Treat
  omitted metadata and `{}` equivalently. Sort JSON object keys recursively for fingerprinting
  and preserve array order and value types. Keep `content_hash` based on indexed text bytes.
  An image asset has a separate byte hash under ADR-0015.
- Completed identical requests return `unchanged` with the original receipt fields, without
  embedding again. Reusing a key for different content or metadata returns `source_conflict`.
  Check fingerprints and existing documents, including tombstones. A revision uses a new key;
  updates to logical sources require a later contract.
- Text and image memories share the source-key namespace. Source kind is fingerprinted,
  so a text source cannot be silently changed into an image source.
- Before new indexing, compare embedding model name and dimension to the active index.
  Reject incompatibility rather than mixing mock/ONNX vectors or silently re-embedding.
  A completed retry returns its existing receipt before checking the configured embedder.
- Filesystem deletion reconciliation selects filesystem sources only. Logical MCP/image
  sources cannot be deleted by scanning a folder, including broad roots such as `.`.

Return a receipt containing document/source IDs, content hash, chunk count, model name and
dimension, and commit timestamp. Provide a read-only exact lookup by source key. Resolve
the canonical source and return its receipt and document metadata only when the document is
active and its receipt is complete. Store a bounded preview from the first indexed chunk
with the receipt so lookup can use the existing metadata handle without embedding or
opening additional stores. Return no match if the key has no active, committed document.
Journal states are internal. Report a failure to the calling client with a sanitized code
and an indication of whether an identical retry is appropriate.

### Durable creation and recovery

1. Validate and normalize input; check completed receipts and conflicts. Parse, chunk, and
   embed without changing committed indexes. For a new request, reject an empty chunk set.
2. Before any index write, commit a `pending` journal entry containing the fingerprint,
   normalized input including custom metadata, model/chunking configuration, and intended
   chunk IDs. Extend the document repository with journal/receipt tables using its
   existing database handle.
3. Write vectors, the chunk map, and keyword records. Keep the new document inactive until
   all required artifacts exist. Commit document activation and receipt completion together
   in one document-repository transaction. Do not report success earlier.
4. On interruption, repair using intended chunk IDs even if the sidecar is absent. Add
   idempotent delete-by-chunk-ID behind a storage port and deduplicate chunk-map entries.
   Clean only the unfinished operation's artifacts and replay the journal. Never apply
   the unchanged-content shortcut to unfinished operations.
5. Persist enough configuration to reproduce intended chunks. If the required embedding
   model/configuration is unavailable, retain the operation as failed and require repair
   with that configuration; do not replay it under a different model silently.
6. After acquiring the lock, check the journal before serving a read or write. The next
   operation can recover unfinished work even if it runs in a different process. If
   recovery cannot finish, record the reason, close the handles, and release the lock.
   Return `recovery_required` to the caller. Every adapter uses this check.

Retain staged input until completion; discard it afterward while keeping fingerprint and
receipt. A canceled queued request may be discarded. Once persistence starts, finish it
or leave a recoverable journal entry. Disconnects and lost responses do not prove rollback.

This decision guarantees recoverable creation, not cross-store ACID transactions or atomic
replacement of existing CLI documents. A future replacement contract needs its own recovery
policy. Image ingestion extends the journal with asset staging under ADR-0015.

### Temporary store access lock

1. Resolve and canonicalize the store directory, including filesystem aliases. Use one
   stable lock file in that directory. Leave the file in place across operations and
   process exits. The OS lock on its open handle controls access.
2. Acquire the exclusive OS lock before opening store handles, for reads and writes.
   Wait up to five seconds for access. If the wait expires, return retryable
   `store_in_use` without opening the store. The timeout limits the waiter's delay.
3. Open the handles needed by the operation and run the recovery check. Hold the lock
   across related reads, index writes, and commits. A CLI folder ingest includes its
   missing-file reconciliation in the same lock scope. Image and link operations use
   the same lock for their related metadata and asset access.
4. Finish the operation and close all store handles before releasing the lock.
   Release it on success and error. The OS releases it if the process exits. The next
   operation opens fresh handles and reads the state committed by the previous operation.

Use this sequence for every CLI command and adapter that accesses the shared store.
The MCP worker can keep the embedder warm between calls. It opens database and asset
handles for the duration of a call and releases them when the call finishes. An idle MCP
session holds no store lock. Prepare inputs and format outputs outside the lock where
possible. Run store-dependent validation under the lock.

Deploy compatible CLI and MCP builds together for a shared store. The lock coordinates
processes that follow this policy. Reopening embedded indexes adds work per operation.
Measure the cost with a representative store, including the wait during a long CLI ingest.

## Alternatives considered

| Option | Reason not selected |
|--------|---------------------|
| Call folder ingestion on a temporary directory | Couples one addition to folder reconciliation and filesystem lifecycle. |
| Check only the document content hash | Misses metadata changes and cannot distinguish completed from partial writes. |
| Retry from the chunk sidecar alone | Misses vectors written before the sidecar commit. |
| Implement retries in MCP handlers | Duplicates business behavior across transports and loses durable state on restart. |
| Hold the lock for the lifetime of the MCP server | An idle MCP session would block CLI access. |
| Open independent handles without a shared lock | Leaves cross-store reads, writes, and recovery uncoordinated. |

## Consequences

- **+** Text, image, CLI, and later API adapters can reuse one processing pipeline.
- **+** Ambiguous responses can be retried without duplicate documents or premature success.
- **+** CLI commands and MCP calls can use the same store during an open MCP session.
- **-** Same-store operations serialize. Long operations can make callers wait or retry,
  and each operation reopens the handles it needs.
- **-** Unrecoverable journaled creates block normal data access until repaired. Existing
  CLI replacement semantics remain a separate concern.

## Acceptance criteria for implementation

1. Identical retries before/after restart or response loss return the committed receipt
   and keep chunk counts stable; changed metadata, content, or source kind conflicts.
   Exact lookup returns the active document and receipt after commit and restart, and
   excludes absent or deleted memories.
2. Inject failure between every write, including vectors and sidecar, and prove repair
   produces one complete document without orphan vectors, duplicate mappings, or early visibility.
3. Missing model/configuration preserves the failed journal and blocks normal data access;
   restoring the required configuration permits deterministic repair.
4. While an MCP server remains connected, an idle session permits CLI reads and ingestion.
   When operations overlap, the waiting process acquires the lock after the first closes
   its handles, or returns retryable `store_in_use` after the wait limit. Test contention in
   both directions and with aliased paths. A process crash releases the lock. The next
   operation in either process repairs any unfinished journal before serving data.
5. Folder ingestion preserves logical sources; existing CLI force/dry-run, update/deletion,
   retrieval, and evaluation regressions remain covered using offline mocks.
6. Custom metadata retains its JSON types in document lookup and RuVector after recovery
   and restart. A client field named `document_id` stays inside `user_metadata`. Reordering
   object keys gives the original receipt, while changing a metadata value or type under
   the same source key returns `source_conflict`.
