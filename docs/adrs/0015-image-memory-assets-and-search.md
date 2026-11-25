# ADR-0015: Preserve original images and index their text representations

- **Status:** Proposed
- **Date:** 2026-11-12
- **Milestone:** Image memory
- **Context refs:** [ADR-0009](0009-hybrid-search-rrf.md),
  [ADR-0014](0014-durable-single-document-ingestion.md),
  [ADR-0016](0016-text-image-attachments.md)

## Context

Engineering memories include architecture diagrams, error screenshots, charts, and photos
of whiteboards. Keeping only a description loses visual evidence; keeping only a filename
or external URL does not preserve the image. The current parsers and MiniLM embedding port
accept text, so image bytes cannot simply enter the existing embedding pipeline.

Users need to search for an image by meaning or visible text, then inspect the original.
That requirement is independent of MCP and independent of attaching an image to a text note.

## Decision

Preserve original encoded image files in managed local asset storage and index a linked
text representation through the existing ingestion and hybrid-search pipeline. Start with
descriptions, extracted text (OCR), and user context; defer image embeddings until retrieval
evaluation demonstrates that text representations miss important queries.

### Asset and memory records

| Record | Fields and responsibility |
|--------|---------------------------|
| `ImageAsset` | `asset_id`, full BLAKE3 hash of original bytes, verified MIME type, byte size, width, height, managed relative storage key, creation time. Immutable and deduplicated by byte hash. |
| Image memory document | Existing document fields plus `source_kind: image`, `asset_id`, description, optional OCR/context, provenance per supplied field, representation version. Owns the searchable text and chunks. |

Store assets under `.tovli/assets/<full-hash>.<verified-extension>`. Keep compressed PNG,
JPEG, or WebP originals byte-for-byte; “original” does not mean uncompressed pixel arrays.
Do not put base64 blobs into searchable text, vector metadata, or the keyword index.
Original filenames are display metadata, never storage paths. Asset identity uses the full
hash rather than the shorter existing document-ID suffix.

An image memory is identified by the logical source key from ADR-0014, not the asset hash.
Two distinct memories may describe the same image differently and share one immutable blob.
The asset's byte hash and the image document's text hash serve different purposes; both
appear in its receipt. Deduplication saves bytes, not the user context of distinct memories.
Existing text documents deserialize with a default `source_kind: text`; image-only fields
are optional so existing store records remain readable.

### Searchable representation

Require a nonblank description and accept optional OCR text, user context, title, project,
and tags. An example representation is:

```text
Title: Payment retry architecture
Description: Payment API sends jobs through a queue to a worker; failed jobs return to the queue.
Extracted text: PaymentService; timeout=30s; retries=3
Context: Proposed design saved to explain duplicate payments during the November incident.
```

Build versioned text with explicit field labels, chunk it, embed it with the active text
model, and write the full representation into the keyword index. Store `asset_id` and
source kind in chunk metadata so every image hit can resolve the original. The first
version searches description/OCR/context, not image pixels. An image need not contain
legible text; absence of OCR is valid and is not a failed ingestion.

The AI client or user supplies derived text in this phase. Record each field's origin as
user-provided, client-generated, OCR-derived, or unknown, and generator/model/version when
available. Such provenance is a recorded assertion, not verified authorship. Keep OCR
separate from interpretation. Generated descriptions can omit or invent details; the
original image remains available for inspection and later reprocessing.

Fingerprint the original byte hash, all supplied semantic fields and provenance, kind,
and canonical defaults under ADR-0014. A completed identical retry returns its stored
receipt. Changed descriptions or context under the same key conflict; a new key can create
a distinct memory referencing the same asset. Do not silently rewrite another memory's text.

### Retrieval and relationships

Existing vector/keyword/hybrid search ranks image text chunks alongside ordinary text,
using the same metadata filters and score semantics. Return source kind, image document
ID, asset ID, preview, and project/tags. Load image bytes only on explicit fetch; a result's
URI or ID alone does not send an image to a vision model. A missing managed file returns
`asset_missing` and marks the result unavailable rather than substituting its description.

[ADR-0016](0016-text-image-attachments.md) owns attachment relationships and reverse lookup.
An image memory can exist on its own. Linking it to a note neither copies its blob nor
requires embedding the image again. Text search over the linked note can discover its
attachments even when the image's own description does not match the query.

### Durable storage and input limits

Extend ADR-0014's journal with the asset hash and staged-file location. Decode and verify
supported image bytes before publication; validate MIME type, dimensions, size, and pixel
limits from ADR-0013. Resolve all generated storage paths within the managed root, prevent
symlink/reparse-point escapes, and never fetch client paths or URLs implicitly.

Write and flush a staged file on the same filesystem, persist the pending operation, then
publish the immutable asset with an atomic rename and platform-appropriate durability.
Verify an existing asset against its full hash before reuse. Ensure the final file exists
before activating its image memory. Commit the asset record, document activation, and
receipt together in the metadata database after all required indexes are written.

Recovery recognizes already-published files by their hashes and replays unfinished index
writes. Never remove a reused blob while repairing one memory. Temporary files or final
files without a committed record may remain after a crash; reconcile them against journals
and committed references before cleanup. Missing or corrupt committed assets require
repair and must not be reported as successfully available.

Backups include the asset directory and metadata/index state captured under the same
temporary store lock in ADR-0014. Soft-deleting one memory or removing a link does not
delete shared bytes. Automatic asset garbage collection and derived thumbnail generation are deferred;
retaining an orphan temporarily is preferable to deleting another memory's original.

### Future visual search

Evaluate representative screenshots/diagrams and queries before adding a multimodal model.
If needed, add a separate image embedding index with a compatible text encoder for queries.
Image vectors and existing MiniLM text vectors must not be compared or mixed merely because
their dimensions happen to match. Any fusion with existing search needs an evaluated ranking
decision. Preserved originals allow later re-description, OCR, and embedding without reupload.

## Alternatives considered

| Option | Reason not selected |
|--------|---------------------|
| Keep descriptions only | Loses evidence and prevents later inspection or improved extraction. |
| Keep filenames or remote URLs only | References can disappear or change and do not preserve the supplied bytes. |
| Embed base64 as text | Encoded bytes do not provide a meaningful text description for MiniLM. |
| Store a blob inside every vector record | Duplicates originals across chunks and couples binary storage to retrieval. |
| Require image embeddings immediately | Adds model and index complexity before text-based retrieval has been evaluated. |

## Consequences

- **+** Images remain inspectable, while descriptions/OCR/context reuse existing search.
- **+** Byte deduplication and independent document identity preserve multiple contexts
  without copying originals. Models and representations can evolve later.
- **-** Backups and recovery span managed files as well as database indexes.
- **-** Search recall depends on supplied text; visual/spatial details can be missed.
- **-** Originals may contain sensitive metadata as well as visible content. Byte-preserving
  storage retains that metadata; any redacted derivative must be an explicit separate asset.

## Acceptance criteria for implementation

1. Fetching a committed asset returns the original bytes/hash after restart and backup
   restore; different memories of identical bytes share one stored blob.
2. Semantic-description queries and exact OCR queries find image memories under the same
   project/tag/source filtering and ranking contracts as text memories.
3. Invalid images, MIME mismatches, size/pixel limits, path escapes, and missing/corrupt
   files return explicit failures. Tests need no vision API or network.
4. Failure injection across staging, publication, index writes, and metadata commit proves
   recovery avoids premature success and never deletes another memory's shared original.
5. Existing text records remain readable; source-kind and asset metadata are propagated
   through retrieval. Description-only and text-plus-OCR recall are measured on image queries.
