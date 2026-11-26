# ADR-0016: Link text and image memories through explicit attachment records

- **Status:** Proposed
- **Date:** 2026-11-12
- **Milestone:** Image memory
- **Context refs:** [ADR-0014](0014-durable-single-document-ingestion.md),
  [ADR-0015](0015-image-memory-assets-and-search.md),
  `src/retrieval/domain/retrieval_result.rs`

## Context

A text note may need a diagram and several screenshots. The same image can support multiple
notes, with a different explanation in each. Embedding image bytes inside each note duplicates
storage, while an ordinary Markdown path does not establish a managed relationship or reverse
lookup. Chunk IDs can also change when a filesystem document is edited and re-ingested.

Asset preservation and image-text search are defined by ADR-0015. This ADR decides how text
and image memories relate and how those relationships survive changes to their endpoints.

## Decision

Represent attachments as explicit many-to-many records between a text document and an image
memory document. Store contextual captions on the relationship. Resolve the preserved blob
through the image document's `asset_id`; adding a link does not copy or re-embed that blob.

### Relationship model

| Field | Meaning |
|-------|---------|
| `link_id` | Stable ID derived from the canonical endpoint/anchor tuple, with a uniqueness constraint on that tuple |
| `document_id` | Existing text document receiving the attachment |
| `image_document_id` | Existing image memory document from ADR-0015 |
| `asset_id` | Resolved from the image document; returned with the link, not accepted as an independent conflicting endpoint |
| `chunk_id` | Optional anchor within the text document; absent means a document-level attachment |
| `caption` | Optional contextual explanation, at most 2,048 characters |
| `created_at` | Original creation timestamp |
| `state`, `suppressed_at`, `suppression_reason` | Persist active/suppressed lifecycle state independently of the chunk's current anchor state |

The tuple is `(document_id, image_document_id, chunk_id-or-document-scope)`. One document
can link multiple images, and one image memory can link multiple documents. An image blob
can also be shared by distinct image memories; retaining `image_document_id` identifies
which description/provenance the link refers to. Return endpoint IDs and asset ID together.

For example, a note about duplicate payments can link the payment architecture image with
the caption “The retry loop that allowed duplicate jobs.” A migration note can link the same
image memory with “Queue topology before the migration.” Neither caption overwrites the
image memory's general description or the other link's caption.

### Creation and consistency

Validate both endpoints are committed and active: the first must be text and the second
an image memory with an available asset. If supplied, the chunk must currently belong to
the text document. Reject missing/deleted endpoints, wrong kinds, and foreign chunks.
Link creation is a separate operation after both ingestion receipts complete. This lets
a client attach an existing image to another note and retry a failed link using the
committed IDs. A client can perform the necessary tool calls from one user request.
Each save and the link commit independently, so the client can resume at the failed step.

Persist links and their forward/reverse lookup indexes in the document metadata database
using the handle opened under the temporary store lock. Expose writes through an application
service and reads through a read-only lookup port. Check endpoints and commit the link/index
entries in one metadata transaction. Hold the lock from chunk validation through commit
to prevent concurrent ingestion from changing the anchor during link creation.

An identical endpoint/anchor tuple and caption returns `unchanged` with the original ID and
timestamp. A different caption for an existing tuple returns `link_conflict`; create-only
ingestion does not silently revise existing captions. Different anchors create distinct
links. A retry against a suppressed relationship returns `link_inactive`, not reactivation.
Caption changes and explicit unlinking need a later mutation contract.

Links are separate from document content and ingestion fingerprints. Adding an attachment
does not modify the text document's content hash, re-embed its chunks, or violate its
create-only receipt. Initial captions enrich displayed context but are not separately
indexed; searching by a link caption alone is not promised in this phase.

### Search and navigation

After ranking text chunks, a presentation/application query service resolves active links
for the selected documents/chunks. Return lightweight image references and captions rather
than loading bytes. Offer paginated expansion instead of putting all attachments on every
search hit; keep ranking and candidate generation under the existing read-only Retrieval
contract.

An image search hit includes its image document ID and asset ID. Reverse lookup returns the
active text memories linked to that image document, with their contextual captions. A separate
lookup by asset can aggregate distinct image memories later; it must not silently merge their
descriptions or provenance.

For a text chunk hit, include document-level links and links anchored to that chunk. Full
document expansion lists all active links and their anchor state (`document`, `current`, or
`stale`). Apply requested metadata filters
to related target documents as well as search candidates; never append unrelated filtered-out
notes during enrichment. Project and tags are retrieval filters, not an access-control system.
Enrichment adds neither a retrieval score nor an automatic relevance boost, and it does not
turn a caption into evidence that an image was visually inspected.

### Re-ingestion and deletion

Document-level attachments survive content changes while the stable document ID remains.
For chunk-level attachments, resolve the stored chunk ID against the current document:
if absent, return `anchor_state: stale` rather than guessing a replacement chunk. Keep the
link available in full document expansion with the stale warning; it is not presented as
attached to a current chunk. Re-ingestion must expose a current chunk-membership lookup so
this validation does not rely only on vector previews.

When deleting a text/image endpoint, mark its relationships suppressed in the same metadata
transaction and retain their records for history. Ordinary expansion and reverse lookup
exclude suppressed relationships and independently validate that endpoints remain active.
Do not silently reactivate a suppressed relationship when an endpoint is restored; require
explicit revalidation in a future restoration workflow. Deleted or stale links never cause
deletion of an image asset.
Asset cleanup follows ADR-0015's shared-reference policy and is deferred in the first release.

## Alternatives considered

| Option | Reason not selected |
|--------|---------------------|
| One image field on a text document | Cannot represent multiple images or reuse with separate contextual captions. |
| Copy image bytes into every text memory | Duplicates storage and entangles attachment changes with document ingestion. |
| Infer links from Markdown paths | Paths can break and do not provide validated endpoints, reverse lookup, or stable identity. |
| Store one global caption on the image | Loses the reason the same image was attached to different notes. |
| Automatically remap missing chunk anchors | Can attach visual evidence to the wrong paragraph after an edit. |

## Consequences

- **+** Users can navigate text-to-image and image-to-text without duplicating originals.
- **+** Each note preserves why an image matters in that context, independently of general
  image description and ingestion receipts.
- **-** Relationship indexes, endpoint validation, stale anchors, and lifecycle handling
  are new application work. Caption-only search and link edits remain future extensions.
- **-** A client must ingest each endpoint before linking; adding three objects is not an
  atomic bundle, so clients use receipts and idempotent retries between steps.

## Acceptance criteria for implementation

1. One text links multiple images and one image memory links multiple texts, with distinct
   per-link captions and one copy of each original blob.
2. Exact retries preserve link counts/IDs/timestamps; changed captions conflict without
   modifying the text hash or committed ingestion receipt.
3. Missing/deleted endpoints, wrong source kinds, and chunks from another document fail.
   A crash cannot leave forward/reverse indexes disagreeing with committed links.
4. Text results expand to linked images; image results expand to linked text with target
   filters respected, stable pagination, and unchanged search ranks/scores.
5. Re-ingestion preserves document links and marks missing chunk anchors stale. Endpoint
   deletion suppresses relationships without deleting shared assets or losing history;
   restoration cannot silently reattach stale evidence.
