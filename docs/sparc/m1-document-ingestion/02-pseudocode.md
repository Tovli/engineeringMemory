# SPARC Phase 2 — Pseudocode: M1 Document Ingestion

Covers every acceptance criterion from `01-specification.md`. Language-neutral but shaped for
Rust. Error paths are explicit; complexity annotated. No real code yet.

## Data structures
```
Config {                         # tovli.config.json (FR-CHK-002, R7)
  target_chunk_tokens=500, max_chunk_tokens=800, overlap_tokens=80,
  embedding_model="all-MiniLM-L6-v2", embedding_dim=384,
  vector_store_path, doc_db_path
}
DocumentRecord {                 # redb sidecar (D-PERSIST), PRD §11.1
  id, source_path, file_name, ext, content_hash,
  title?, project?, tags[], created_at, updated_at, deleted_at?
}
ChunkRecord {                    # PRD §11.2 + FR-EMB-002
  id, document_id, chunk_index, heading_path[], content, preview,
  content_hash, token_count,
  embedding_model, embedding_dim, embedding_created_at, metadata{}
}
```

## Traits (seams — C5, R2, R9)
```
trait Parser   { fn parse(&self, text) -> ParsedDoc }         # md/txt/json/yaml impls
trait Chunker  { fn chunk(&self, ParsedDoc, Config) -> [Chunk] }
trait Embedder { fn dim(&self) -> usize; fn embed_batch(&self, [text]) -> Result<[Vec<f32>]> }
# VectorStore: reused from M0 (RuVector backend)
# IngestionService orchestrates the above; CLI handler only parses args + calls it.
```

## Main pipeline — `ingest(path, opts)`
```
config   = load_config()                                   # O(1)
embedder = build_embedder(config)        # ONNX MiniLM; FATAL on load err (NFR §9.5)
store    = vector_store.open(config.path, embedder.dim())  # AC-8 dim is fixed here
docdb    = doc_db.open(config.doc_db_path)
files    = scan(path)                    # recursive; guard symlink loops/depth (E6). O(F)
summary  = Summary::new();  seen = {}

for file in files:                                         # O(F)
  if file.ext not in SUPPORTED:  summary.skip(file,"unsupported"); continue   # AC-2, R2
  bytes = read(file)            or { summary.error(file,"io"); continue }      # §9.5 keep going
  text  = utf8(bytes)           or { summary.skip(file,"non-utf8"); continue } # E3
  fhash = blake3(text)                                                        # R3, C6 (D-HASH)

  doc = docdb.get_by_path(file.path)
  if doc and doc.content_hash == fhash and not opts.force:                     # AC-3
      summary.unchanged += 1; seen.add(doc.id); continue                       # R4 skip, no re-embed
  if doc:  delete_chunks(store, docdb, doc.id)            # modified → drop old chunks (AC-4)
  doc_id = doc.id if doc else new_id()

  parsed = parser_for(file.ext).parse(text)                                    # R2
  chunks = chunker.chunk(parsed, config)                 # R6; O(content_len)
  if chunks.is_empty():                                  # E1 empty/whitespace
      upsert_doc_only(docdb, doc_id, file, fhash, opts); summary.empty(file); seen.add(doc_id); continue

  vectors = embedder.embed_batch(chunks.map(.content))   # dominates cost; per-file err → summary.error, continue unless --strict
  assert embedder.dim() == store.dim()  else FATAL DimMismatch                 # AC-8
  records = build_chunk_records(doc_id, chunks, vectors, config, now())        # AC-6 fields

  if opts.dry_run: summary.would_ingest(file, records.len()); seen.add(doc_id); continue   # AC-7 no writes

  docdb.upsert_document(doc_id, file, fhash, opts.project, opts.tags, now())    # R5
  docdb.put_chunks(records)
  store.upsert(records -> [{id, vector, metadata}])      # R11 via VectorStore seam
  summary.ingested(file, records.len());  seen.add(doc_id)

# Deletions (R4, E5): active docs under `path` not seen this run
for doc in docdb.active_under(path):
  if doc.id not in seen:  docdb.mark_deleted(doc); store.remove(doc.id); summary.deleted += 1

summary.print(config)     # AC-1 counts (scanned/ingested/unchanged/skipped/chunks) + chunk config (R7)
return Ok(summary)
```

## Markdown-aware chunking — `chunker.chunk` (R6, AC-5, E2)
```
if md:
  walk tokens, maintaining a heading-path stack (h1>h2>h3...)
  treat each ```fenced``` block and each table as ATOMIC (never split — AC-5)
  pack consecutive units into chunks up to target_tokens, hard cap max_tokens,
    carry overlap_tokens of trailing text into the next chunk
  if a single atomic unit > max_tokens: emit it whole, flag oversized=true (E2)
  each chunk records its heading_path; preview = first N chars, deterministic (R8)
else (txt/json/yaml):
  pack_into_chunks(parsed.text, target, max, overlap); heading_path = []
# token_count via char/word heuristic (D-TOK) until a real tokenizer is added
```

## Error path summary (NFR §9.5)
| Failure | Behavior |
|---|---|
| file read / permission | record `error`, continue run |
| non-UTF8 / binary | `skip` with warning (E3) |
| parser error | `skip` file with reason |
| embedder model load | **FATAL** — clear message + retry guidance |
| embed inference (per file) | record `error`, continue unless `--strict` |
| embedding dim ≠ index dim | **FATAL** `DimMismatch` (AC-8) |
| store write | record `error`, continue (or abort per config) |

## Complexity
`O(F + Σ content_len + N_chunks · embed_cost)`; embedding dominates. Idempotent hash-skip
removes unchanged files from the embed cost on re-runs (AC-3). Target: 100 md files < 5 min (C4).

## Phase-2 Gate (to advance → Architecture)
- [x] Pseudocode covers all 8 acceptance criteria (mapped inline)
- [x] Error paths explicit (table above)
- [x] Complexity annotated
- [ ] **Blocked on:** ONNX toolchain verified (task #13) before committing the Architecture to ort/MiniLM
