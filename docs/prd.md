# **PRD: tovli — RuVector-Based Technical Memory Assistant**

## **1\. Product Summary**

tovli is a local-first technical knowledge assistant that helps an engineer or engineering manager search, retrieve, and reason over their own technical documents.

The system ingests documents such as Markdown files, technical conventions, architecture notes, incident summaries, Dockerfile reviews, GitHub Action errors, PRDs, and decision records. It chunks the documents, generates embeddings, stores them in RuVector, retrieves relevant chunks for a user question, and optionally generates cited answers using an LLM.

The main purpose of this project is educational: to learn how to work effectively with RuVector and vector-based retrieval systems in a realistic product context.

The product should start as a CLI tool, evolve into a local API, and only later add a bot or web interface.

---

## **2\. Problem Statement**

Engineering knowledge is often scattered across Markdown files, PDFs, chats, PRDs, GitHub issues, architecture documents, and ad hoc notes. Simple keyword search is often insufficient because users ask conceptual questions rather than exact-match queries.

Examples:

* “What do our docs say about separating deployment boundaries from component boundaries?”  
* “Find past issues similar to this Azure Function deployment error.”  
* “What are the main React Native conventions we agreed on?”  
* “Which documents mention architecture layering?”  
* “What did we decide about Firebase auth migration?”  
* “Which previous errors look similar to this npm ci dependency problem?”

The product should help retrieve the most relevant internal knowledge, expose the sources, and make the quality of retrieval measurable.

---

## **3\. Goals**

The primary goal is to build a working RuVector-based RAG system that teaches the full retrieval lifecycle.

The product should teach:

* How to ingest documents.  
* How to chunk documents correctly.  
* How to generate and store embeddings.  
* How to create vector indexes.  
* How vector search differs from keyword search.  
* How metadata filters improve search.  
* How hybrid search improves retrieval quality.  
* How to evaluate retrieval quality.  
* How to generate cited answers.  
* How to log and debug bad answers.  
* How feedback can improve future retrieval.

The product should be useful enough for real personal engineering work, but small enough to complete incrementally.

---

## **4\. Non-Goals**

The first version will not be a general-purpose enterprise knowledge platform.

Out of scope for the initial milestones:

* Multi-user SaaS.  
* Complex permission model.  
* Full Google Drive / Notion / Slack integration.  
* Autonomous agent workflows.  
* Long-running background workers beyond local ingestion.  
* Real-time sync from external systems.  
* Fine-tuning embedding models.  
* Production-grade web authentication.  
* Automatic document classification using an LLM.  
* Fully autonomous decision-making.

The system may later support these capabilities, but they should not be included in the first learning-focused version.

---

## **5\. Target Users**

### **Primary User**

A technical user who wants to learn RuVector by building a realistic retrieval system.

This user is comfortable with TypeScript, CLI tools, Docker, SQL, and engineering documents.

### **Secondary User**

An engineering manager or tech lead who wants a searchable second brain for technical decisions, architectural notes, operational issues, and project history.

---

## **6\. Core Use Cases**

### **Use Case 1: Ingest Local Documents**

The user points the system to a local folder.

Example:

tovli ingest ./docs

The system reads supported files, splits them into chunks, generates embeddings, stores the chunks in RuVector, and records metadata about each source file.

---

### **Use Case 2: Search Without LLM**

The user asks a question and receives the top matching chunks.

Example:

tovli search "What are our architecture layering rules?"

The system returns:

* Chunk title.  
* Source file.  
* Similarity score.  
* Short preview.  
* Chunk ID.  
* Metadata.

This mode is required before adding answer generation.

---

### **Use Case 3: Ask With Cited Answer**

The user asks a natural-language question.

Example:

tovli ask "What do our docs say about architecture boundaries?"

The system retrieves relevant chunks, sends them to an LLM, and returns a concise answer with citations.

The answer must never hide its sources.

---

### **Use Case 4: Find Similar Errors**

The user pastes an error message.

Example:

tovli similar "./error.txt"

The system retrieves similar previous errors, notes, or troubleshooting documents.

This use case is especially useful for build errors, Docker issues, GitHub Action failures, Azure deployment issues, and dependency conflicts.

---

### **Use Case 5: Evaluate Retrieval Quality**

The user creates a test set of questions and expected source chunks.

Example:

tovli eval ./eval/questions.json

The system runs all test questions and calculates retrieval quality metrics.

---

### **Use Case 6: Give Feedback**

The user marks retrieved chunks as useful or not useful.

Example:

tovli feedback \--question-id q\_123 \--good chunk\_7 \--bad chunk\_2

The system stores this feedback for analysis and later ranking improvements.

---

### **Use Case 7: Ask Through Telegram or Slack**

The user asks questions through a bot.

Example:

/ask What do we know about Azure Function 403 zipDeploy errors?

This should only be added after the CLI and retrieval evaluation are stable.

---

## **7\. Product Principles**

### **Retrieval Before Generation**

The system must first prove that it can retrieve the correct source chunks before it generates answers.

LLM-based answer generation should not be used to hide weak retrieval.

### **Sources Are Mandatory**

Every generated answer must include source references. If the system cannot find reliable sources, it must say that.

### **Observability Is a Product Feature**

The product must make retrieval visible and debuggable. The user should be able to see which chunks were retrieved, why they were retrieved, what scores they received, and whether the final answer used them.

### **Local-First**

The default version should run locally using Docker and local files. External LLMs may be optional, but the system should not require a cloud product just to test retrieval.

### **Small Interfaces First**

Start with CLI. Add API only after the CLI works. Add UI or bot only after the API works.

---

## **8\. Functional Requirements**

## **8.1 Document Ingestion**

### **FR-ING-001: Ingest Folder**

The system shall allow the user to ingest all supported documents from a local folder.

Command:

tovli ingest ./docs

Acceptance criteria:

* The command scans the folder recursively.  
* The command detects supported file types.  
* The command creates document records.  
* The command creates chunk records.  
* The command creates embeddings for each chunk.  
* The command stores embeddings in RuVector.  
* The command reports how many files and chunks were ingested.

---

### **FR-ING-002: Supported File Types**

Initial supported file types:

* `.md`  
* `.txt`  
* `.json`  
* `.yaml`  
* `.yml`

Later supported file types:

* `.pdf`  
* `.docx`  
* `.html`  
* `.csv`

Acceptance criteria:

* Unsupported files are skipped with a clear warning.  
* The ingestion summary lists skipped files.  
* File type parsing logic is isolated behind a parser interface.

---

### **FR-ING-003: Idempotent Ingestion**

The system shall not duplicate chunks when the same file is ingested multiple times without changes.

Acceptance criteria:

* Each source file has a content hash.  
* Each chunk has a chunk hash.  
* Re-ingesting unchanged files does not create duplicate chunks.  
* Modified files are re-chunked and re-indexed.

---

### **FR-ING-004: Incremental Re-Indexing**

The system shall re-index only changed files.

Acceptance criteria:

* New files are added.  
* Modified files are reprocessed.  
* Deleted files are marked as deleted or removed from the active index.  
* Unchanged files are skipped.

---

### **FR-ING-005: Metadata Extraction**

The system shall extract basic metadata for each document and chunk.

Required metadata:

* Source file path.  
* File name.  
* File extension.  
* Content hash.  
* Created timestamp.  
* Updated timestamp.  
* Chunk index.  
* Chunk character length.  
* Optional title.  
* Optional tags.  
* Optional project name.  
* Optional topic.

Acceptance criteria:

* Metadata is queryable.  
* Metadata can be used for filtering search results.  
* Metadata is included in search output.

---

## **8.2 Chunking**

### **FR-CHK-001: Markdown-Aware Chunking**

The system shall split Markdown documents using headings where possible.

Acceptance criteria:

* Top-level and sub-level headings are preserved.  
* Each chunk stores its heading path.  
* Chunks do not split code blocks in the middle.  
* Chunks do not split tables in the middle when avoidable.

---

### **FR-CHK-002: Configurable Chunk Size**

The system shall allow chunk size configuration.

Default values:

{  
  "targetChunkTokens": 500,  
  "maxChunkTokens": 800,  
  "overlapTokens": 80  
}

Acceptance criteria:

* The user can change chunk settings in a config file.  
* The ingestion command reports the chunking configuration used.  
* Chunks larger than the maximum are split safely.

---

### **FR-CHK-003: Chunk Preview**

The system shall store a short preview for each chunk.

Acceptance criteria:

* Preview is shown in search results.  
* Preview is generated deterministically.  
* Preview does not require an LLM.

---

## **8.3 Embeddings**

### **FR-EMB-001: Embedding Provider Interface**

The system shall support multiple embedding providers through an abstraction.

Initial providers:

* RuVector/Postgres local embedding functions, if available and stable.  
* External embedding provider fallback.  
* Mock deterministic provider for tests.

Acceptance criteria:

* Embedding logic is not coupled to ingestion logic.  
* The selected embedding provider is recorded in metadata.  
* The embedding model name and dimension are stored.  
* The system prevents mixing embeddings with incompatible dimensions in the same index.

---

### **FR-EMB-002: Embedding Model Versioning**

The system shall track which embedding model was used for each chunk.

Acceptance criteria:

* Each chunk stores `embedding_model`.  
* Each chunk stores `embedding_dimension`.  
* Each chunk stores `embedding_created_at`.  
* Changing embedding model requires a re-index command.  
* The system warns when the active query model differs from indexed chunks.

---

### **FR-EMB-003: Re-Embedding Command**

The system shall allow re-generating embeddings.

Command:

tovli reembed \--model \<model-name\>

Acceptance criteria:

* The command re-generates embeddings for all active chunks.  
* The command updates the embedding metadata.  
* The command reports progress.  
* The command does not delete source documents.

---

## **8.4 RuVector Storage**

### **FR-DB-001: RuVector-Postgres Storage**

The system shall support RuVector-Postgres as the primary vector store.

Acceptance criteria:

* The system can run with Docker Compose.  
* The system creates required tables.  
* The system creates a vector index.  
* The system supports vector similarity search.  
* The system supports metadata filtering.

---

### **FR-DB-002: Schema Migration**

The system shall include database migrations.

Acceptance criteria:

* Migrations are versioned.  
* Migrations can be run from CLI.  
* A fresh database can be initialized from zero.  
* Existing data is preserved across compatible migrations.

---

### **FR-DB-003: Local Development Setup**

The system shall include a one-command local setup.

Example:

docker compose up \-d  
pnpm install  
pnpm db:migrate

Acceptance criteria:

* A developer can run the system locally.  
* Required environment variables are documented.  
* The README includes setup instructions.  
* Sample documents are included.

---

## **8.5 Search**

### **FR-SRCH-001: Vector Search**

The system shall retrieve the top K chunks by vector similarity.

Command:

tovli search "query text" \--top-k 8

Acceptance criteria:

* The command embeds the query.  
* The command searches RuVector.  
* The command returns top K chunks.  
* Each result includes source, chunk ID, score, and preview.

---

### **FR-SRCH-002: Metadata Filtering**

The system shall support filtering search results by metadata.

Examples:

tovli search "deployment error" \--project flexid  
tovli search "architecture boundaries" \--tag architecture  
tovli search "auth migration" \--source ./docs/auth.md

Acceptance criteria:

* Filters are applied at query time.  
* Search output clearly shows active filters.  
* Empty results are handled gracefully.

---

### **FR-SRCH-003: Hybrid Search**

The system shall support hybrid search using vector similarity and keyword/BM25-style scoring where available.

Command:

tovli search "npm ci typescript peer dependency" \--mode hybrid

Acceptance criteria:

* The system supports `vector`, `keyword`, and `hybrid` modes.  
* The selected mode is shown in output.  
* Hybrid search combines semantic and lexical ranking.  
* The ranking method is documented.

---

### **FR-SRCH-004: Search Explain Mode**

The system shall provide an explain mode for debugging retrieval.

Command:

tovli search "architecture boundaries" \--explain

Acceptance criteria:

* Shows query embedding provider.  
* Shows search mode.  
* Shows filters.  
* Shows score per result.  
* Shows ranking method.  
* Shows why each chunk was eligible.

---

## **8.6 RAG Answer Generation**

### **FR-RAG-001: Ask Command**

The system shall generate an answer from retrieved chunks.

Command:

tovli ask "What do our docs say about architecture boundaries?"

Acceptance criteria:

* The system retrieves relevant chunks.  
* The system sends only retrieved chunks to the LLM.  
* The system generates a concise answer.  
* The answer includes source references.  
* The answer refuses to answer when sources are weak.

---

### **FR-RAG-002: Cited Answers**

Every generated answer shall include citations.

Citation format:

Sources:  
1\. docs/architecture.md\#chunk-12  
2\. docs/react-native-conventions.md\#chunk-4

Acceptance criteria:

* Every factual claim should be grounded in retrieved chunks.  
* The final output lists all used chunks.  
* Retrieved-but-unused chunks are optionally shown in debug mode.

---

### **FR-RAG-003: No-Answer Behavior**

The system shall explicitly state when it cannot answer.

Acceptance criteria:

* If retrieval scores are below threshold, the system says no reliable source was found.  
* If retrieved chunks are contradictory, the system says the sources conflict.  
* If the question is outside the indexed corpus, the system does not hallucinate.

---

### **FR-RAG-004: Prompt Template Versioning**

The system shall version answer-generation prompts.

Acceptance criteria:

* Prompt templates live in source control.  
* Each answer log stores prompt version.  
* Prompt changes can be evaluated against the test set.

---

## **8.7 Evaluation**

### **FR-EVAL-001: Evaluation Dataset**

The system shall support a JSON evaluation dataset.

Example:

\[  
  {  
    "id": "q\_001",  
    "question": "What is our rule about architecture layers?",  
    "expectedChunkIds": \["chunk\_12"\],  
    "expectedSourceFiles": \["docs/architecture.md"\]  
  }  
\]

Acceptance criteria:

* Evaluation questions can be stored in a file.  
* Expected chunks are optional.  
* Expected source files are supported.  
* Evaluation can run without LLM generation.

---

### **FR-EVAL-002: Retrieval Metrics**

The system shall calculate retrieval metrics.

Required metrics:

* Hit@1.  
* Hit@3.  
* Hit@5.  
* MRR.  
* Average retrieval latency.  
* Number of empty result sets.  
* Number of below-threshold results.

Acceptance criteria:

* Metrics are printed after evaluation.  
* Metrics are saved to a JSON report.  
* Evaluation can compare multiple search modes.

---

### **FR-EVAL-003: Regression Testing**

The system shall support retrieval regression tests.

Command:

tovli eval ./eval/questions.json \--fail-below-hit-at-3 0.8

Acceptance criteria:

* CI can run retrieval tests.  
* The command exits with failure when quality drops below threshold.  
* Reports are saved for comparison.

---

## **8.8 Feedback Loop**

### **FR-FB-001: Feedback Capture**

The system shall allow the user to mark retrieved chunks as useful or not useful.

Command:

tovli feedback \--question-id q\_001 \--good chunk\_12 \--bad chunk\_8

Acceptance criteria:

* Feedback is stored.  
* Feedback includes question, query, retrieved chunks, selected chunks, timestamp, and optional note.  
* Feedback can be exported.

---

### **FR-FB-002: Feedback Report**

The system shall provide a report of problematic queries.

Command:

tovli feedback-report

Acceptance criteria:

* Shows questions with bad retrieval.  
* Shows commonly downvoted chunks.  
* Shows queries with no good result.  
* Shows candidate documents that may need re-chunking.

---

### **FR-FB-003: Learning Experiment Layer**

The system may later use feedback to improve ranking.

Possible approaches:

* Boost chunks with positive feedback for similar queries.  
* Penalize chunks with repeated negative feedback.  
* Add query expansion.  
* Add reranking.  
* Use RuVector advanced/self-learning features where stable.

Acceptance criteria:

* Baseline retrieval metrics are captured before experimentation.  
* Any learning feature can be toggled off.  
* Learning features must improve evaluation metrics before becoming default.

---

## **8.9 API**

### **FR-API-001: Local HTTP API**

The system shall expose a local API after the CLI is stable.

Endpoints:

POST /ingest  
POST /search  
POST /ask  
POST /feedback  
GET /documents  
GET /chunks/:id  
GET /eval/reports

Acceptance criteria:

* API uses the same service layer as CLI.  
* API responses include IDs and metadata.  
* API errors are structured.  
* API is documented with OpenAPI.

---

## **8.10 Bot Interface**

### **FR-BOT-001: Telegram Bot**

The system may support Telegram as a late milestone.

Commands:

/ask \<question\>  
/search \<query\>  
/similar \<pasted error\>  
/sources \<last-answer\>  
/feedback good \<chunk-id\>  
/feedback bad \<chunk-id\>

Acceptance criteria:

* Bot calls the local API.  
* Bot does not contain retrieval logic.  
* Bot returns concise answers.  
* Bot includes source references.  
* Bot supports feedback.

---

## **9\. Non-Functional Requirements**

## **9.1 Performance**

Initial local performance targets:

* Ingest 100 Markdown files in under 5 minutes.  
* Search over 5,000 chunks in under 1 second locally.  
* Generate answer in under 10 seconds, excluding slow external LLM delays.  
* Evaluation of 50 questions in under 2 minutes without answer generation.

These are learning targets, not hard production SLAs.

---

## **9.2 Privacy**

The system should default to local document storage.

Requirements:

* Documents are stored locally.  
* Embeddings are stored locally.  
* External LLM calls are optional.  
* When using an external LLM, only retrieved chunks are sent.  
* The user can disable LLM generation entirely.

---

## **9.3 Observability**

The system shall log:

* Ingestion runs.  
* Number of files processed.  
* Number of chunks created.  
* Embedding provider used.  
* Search mode.  
* Query latency.  
* Retrieved chunks.  
* Scores.  
* Answer generation prompt version.  
* Feedback.

Logs should be structured JSON where possible.

---

## **9.4 Maintainability**

Requirements:

* TypeScript strict mode.  
* Clear module boundaries.  
* No retrieval logic inside CLI command handlers.  
* Testable service layer.  
* Database migrations.  
* Config validation.  
* README with local setup.  
* Example documents and evaluation set.

---

## **9.5 Reliability**

Requirements:

* Failed file ingestion should not stop the entire run unless configured.  
* Partial ingestion failures should be reported.  
* Database connection errors should be clear.  
* Embedding provider errors should include retry guidance.  
* Re-indexing should not corrupt existing data.

---

## **10\. Proposed Architecture**

## **10.1 Components**

### **CLI**

Responsible for user commands:

* `ingest`  
* `search`  
* `ask`  
* `eval`  
* `feedback`  
* `reembed`  
* `status`

The CLI should be thin and delegate to application services.

---

### **Application Services**

Core services:

* `DocumentIngestionService`  
* `ChunkingService`  
* `EmbeddingService`  
* `VectorStoreService`  
* `SearchService`  
* `RagAnswerService`  
* `EvaluationService`  
* `FeedbackService`

---

### **Storage**

Primary storage:

* PostgreSQL with RuVector extension.

Tables:

* `documents`  
* `chunks`  
* `embeddings`  
* `queries`  
* `retrieval_runs`  
* `answers`  
* `feedback`  
* `eval_questions`  
* `eval_runs`

---

### **LLM Layer**

Optional component used only for answer generation.

Responsibilities:

* Build prompt from retrieved chunks.  
* Generate answer.  
* Enforce citation requirement.  
* Refuse when context is insufficient.

---

### **Bot/API Layer**

Late-stage layer that calls the same application services.

---

## **11\. Data Model**

## **11.1 Document**

type Document \= {  
  id: string;  
  sourcePath: string;  
  fileName: string;  
  fileExtension: string;  
  contentHash: string;  
  title?: string;  
  project?: string;  
  tags: string\[\];  
  createdAt: string;  
  updatedAt: string;  
  deletedAt?: string;  
};

---

## **11.2 Chunk**

type Chunk \= {  
  id: string;  
  documentId: string;  
  chunkIndex: number;  
  headingPath: string\[\];  
  content: string;  
  preview: string;  
  contentHash: string;  
  tokenCount: number;  
  metadata: Record\<string, string | number | boolean\>;  
  createdAt: string;  
  updatedAt: string;  
};

---

## **11.3 Embedding**

type Embedding \= {  
  id: string;  
  chunkId: string;  
  modelName: string;  
  dimension: number;  
  vector: number\[\];  
  createdAt: string;  
};

---

## **11.4 Query**

type Query \= {  
  id: string;  
  question: string;  
  searchMode: "vector" | "keyword" | "hybrid";  
  filters: Record\<string, unknown\>;  
  embeddingModel?: string;  
  createdAt: string;  
};

---

## **11.5 Retrieval Result**

type RetrievalResult \= {  
  queryId: string;  
  chunkId: string;  
  rank: number;  
  score: number;  
  sourcePath: string;  
  preview: string;  
};

---

## **11.6 Feedback**

type Feedback \= {  
  id: string;  
  queryId: string;  
  chunkId: string;  
  rating: "good" | "bad";  
  note?: string;  
  createdAt: string;  
};

---

## **12\. CLI Requirements**

## **12.1 `init`**

Initializes project config.

tovli init

Creates:

tovli.config.json  
docs/  
eval/

Acceptance criteria:

* Creates default config.  
* Does not overwrite existing config without confirmation.  
* Prints next steps.

---

## **12.2 `ingest`**

tovli ingest ./docs

Options:

\--force  
\--dry-run  
\--project \<project-name\>  
\--tag \<tag\>

Acceptance criteria:

* Supports dry run.  
* Supports project/tag metadata.  
* Prints ingestion summary.

---

## **12.3 `search`**

tovli search "query"

Options:

\--top-k 8  
\--mode vector|keyword|hybrid  
\--project \<project\>  
\--tag \<tag\>  
\--explain

Acceptance criteria:

* Returns ranked chunks.  
* Supports filters.  
* Supports explain mode.

---

## **12.4 `ask`**

tovli ask "question"

Options:

\--top-k 8  
\--mode vector|keyword|hybrid  
\--no-llm  
\--show-context

Acceptance criteria:

* Generates cited answer.  
* Can show retrieved context.  
* Can run retrieval-only mode.

---

## **12.5 `eval`**

tovli eval ./eval/questions.json

Options:

\--mode vector|keyword|hybrid  
\--top-k 3  
\--fail-below-hit-at-3 0.8  
\--output ./eval/report.json

Acceptance criteria:

* Runs evaluation.  
* Prints metrics.  
* Saves JSON report.  
* Can fail CI if threshold is not met.

---

## **12.6 `feedback`**

tovli feedback \--query-id q\_123 \--good chunk\_7  
tovli feedback \--query-id q\_123 \--bad chunk\_2

Acceptance criteria:

* Stores feedback.  
* Validates query and chunk IDs.  
* Supports optional note.

---

## **13\. Milestones**

## **Milestone 0: Technical Spike**

Objective:

Validate that RuVector can be installed locally and queried from TypeScript.

Scope:

* Run RuVector-Postgres using Docker.  
* Create a simple documents table.  
* Insert sample vectors.  
* Query nearest neighbors.  
* Test Node.js connectivity.  
* Document setup problems.

Deliverables:

* `docker-compose.yml`  
* Minimal DB migration.  
* Minimal TypeScript script.  
* Setup notes.

Acceptance criteria:

* Developer can run local RuVector.  
* Developer can insert vectors.  
* Developer can perform similarity search.  
* Setup is documented.

Learning outcome:

Understand RuVector installation, vector column setup, and basic similarity query.

---

## **Milestone 1: Document Ingestion MVP**

Objective:

Build the first usable ingestion pipeline for Markdown and text files.

Scope:

* Folder scanning.  
* Markdown/text parsing.  
* Basic chunking.  
* Content hashing.  
* Document and chunk storage.  
* Embedding generation.  
* Vector storage.

Deliverables:

* `tovli ingest ./docs`  
* Database schema for documents/chunks/embeddings.  
* Basic config file.  
* Sample docs.

Acceptance criteria:

* Ingests at least 20 local Markdown files.  
* Creates chunks.  
* Stores embeddings.  
* Skips unchanged files on second run.  
* Prints ingestion summary.

Learning outcome:

Understand chunking, embedding generation, idempotent ingestion, and vector storage.

---

## **Milestone 2: Retrieval CLI**

Objective:

Search documents using vector similarity.

Scope:

* Query embedding.  
* Top-K vector search.  
* Search result formatting.  
* Metadata filters.  
* Explain mode.

Deliverables:

* `tovli search`  
* Search service.  
* Result formatter.  
* Metadata filter support.

Acceptance criteria:

* User can search by natural language.  
* Search returns ranked chunks.  
* Results include source file and score.  
* Filters work by project/tag/source.  
* Explain mode shows ranking details.

Learning outcome:

Understand retrieval quality, similarity scores, metadata filtering, and debugging.

---

## **Milestone 3: Retrieval Evaluation**

Objective:

Make retrieval quality measurable before adding LLM answers.

Scope:

* Evaluation dataset format.  
* Hit@K metrics.  
* MRR.  
* Latency measurement.  
* JSON reports.  
* CI-compatible failure threshold.

Deliverables:

* `tovli eval`  
* Example evaluation file.  
* Evaluation report format.

Acceptance criteria:

* At least 20 test questions are defined.  
* Hit@3 is calculated.  
* MRR is calculated.  
* Results are saved to JSON.  
* CLI can fail if quality is below threshold.

Target quality:

* Hit@3 should reach at least 80% on the initial curated dataset.

Learning outcome:

Understand that vector search must be measured, not trusted blindly.

---

## **Milestone 4: RAG Answer Generation**

Objective:

Generate answers from retrieved chunks with mandatory citations.

Scope:

* LLM provider abstraction.  
* Prompt template.  
* Context assembly.  
* Source citation formatting.  
* No-answer behavior.  
* Prompt versioning.

Deliverables:

* `tovli ask`  
* Prompt templates.  
* Answer logs.  
* Source citation output.

Acceptance criteria:

* Answers include source references.  
* Weak retrieval produces no-answer response.  
* Retrieved context can be shown with `--show-context`.  
* Prompt version is stored with answer log.

Learning outcome:

Understand how RAG depends on retrieval quality and citation discipline.

---

## **Milestone 5: Hybrid Search**

Objective:

Improve retrieval by combining semantic search and lexical search.

Scope:

* Keyword search mode.  
* Hybrid search mode.  
* Score fusion.  
* Comparison across search modes.  
* Evaluation by mode.

Deliverables:

* `--mode vector`  
* `--mode keyword`  
* `--mode hybrid`  
* Evaluation comparison report.

Acceptance criteria:

* User can select search mode.  
* Hybrid search improves or equals vector-only Hit@3 on the evaluation set.  
* Exact technical terms are better handled.  
* Ranking method is documented.

Learning outcome:

Understand why embeddings alone are often insufficient for technical search.

---

## **Milestone 6: Feedback and Retrieval Debugging**

Objective:

Capture user feedback and identify weak retrieval areas.

Scope:

* Feedback command.  
* Feedback storage.  
* Feedback report.  
* Bad query analysis.  
* Re-chunking recommendations.

Deliverables:

* `tovli feedback`  
* `tovli feedback-report`  
* Feedback tables.  
* Report of problematic queries.

Acceptance criteria:

* User can mark chunks good or bad.  
* Feedback is tied to query ID.  
* Report shows low-quality retrieval patterns.  
* Feedback data can be exported.

Learning outcome:

Understand retrieval observability and feedback loops.

---

## **Milestone 7: Local API**

Objective:

Expose the system through a local HTTP API.

Scope:

* Search endpoint.  
* Ask endpoint.  
* Feedback endpoint.  
* Document listing endpoint.  
* OpenAPI documentation.

Deliverables:

* Local API server.  
* OpenAPI spec.  
* API examples.

Acceptance criteria:

* API calls use the same services as CLI.  
* API returns structured JSON.  
* API supports search, ask, and feedback.  
* CLI continues to work.

Learning outcome:

Understand how to separate retrieval services from interface layers.

---

## **Milestone 8: Telegram Bot**

Objective:

Use the system from a chat interface.

Scope:

* Telegram bot integration.  
* `/ask`  
* `/search`  
* `/similar`  
* `/feedback`  
* Source display.

Deliverables:

* Bot service.  
* Bot command handlers.  
* Bot setup documentation.

Acceptance criteria:

* Bot can answer using local API.  
* Bot includes citations.  
* Bot can capture feedback.  
* Bot does not contain core retrieval logic.

Learning outcome:

Understand how retrieval systems power agent-like interfaces without mixing concerns.

---

## **Milestone 9: Advanced RuVector Experiments**

Objective:

Experiment with RuVector-specific advanced capabilities after the baseline is stable.

Scope:

* RuVector local embeddings, if not already used.  
* RuVector hybrid search functions.  
* RuVector sparse vectors.  
* RuVector self-learning or adaptive features.  
* Quantization.  
* Filtered HNSW behavior.  
* Performance comparison.

Deliverables:

* Experiment notes.  
* Before/after evaluation report.  
* Toggleable feature flags.

Acceptance criteria:

* Each experiment has a baseline.  
* Each experiment is measured.  
* No advanced feature becomes default without improving quality or performance.  
* Findings are documented.

Learning outcome:

Understand RuVector beyond basic vector storage.

---

## **14\. Quality Bar**

The system is considered useful when:

* It can ingest at least 100 technical documents.  
* It can search at least 5,000 chunks locally.  
* Hit@3 is at least 80% on a curated test set.  
* Every generated answer includes citations.  
* Bad retrieval can be debugged.  
* Re-indexing is safe and repeatable.  
* Search can be run in vector, keyword, and hybrid modes.  
* Feedback can be captured and reviewed.

---

## **15\. Risks**

### **Risk 1: RuVector APIs may be unstable**

Mitigation:

* Isolate RuVector behind a `VectorStoreService`.  
* Keep SQL and vector-store-specific code in one module.  
* Avoid coupling the whole app to advanced RuVector features too early.

---

### **Risk 2: Retrieval quality may be poor**

Mitigation:

* Add evaluation before RAG.  
* Tune chunking.  
* Compare vector, keyword, and hybrid search.  
* Add metadata filters.  
* Add feedback reports.

---

### **Risk 3: LLM answers may hallucinate**

Mitigation:

* Require citations.  
* Use no-answer threshold.  
* Show retrieved chunks.  
* Keep prompt strict.  
* Do not generate answers when retrieval is weak.

---

### **Risk 4: Chunking may damage context**

Mitigation:

* Preserve Markdown headings.  
* Avoid splitting code blocks.  
* Use overlap.  
* Track source hierarchy.  
* Review bad retrieval cases.

---

### **Risk 5: Embedding model changes may corrupt quality**

Mitigation:

* Store embedding model and dimension.  
* Prevent mixed-dimension queries.  
* Add explicit re-embedding command.  
* Run evaluation after re-embedding.

---

## **16\. Open Questions**

1. Should the first implementation use RuVector-Postgres only, or also support the Node.js core package?  
2. Which embedding model should be the default?  
3. Should LLM generation use a local model or external provider?  
4. Should PDF support be included early or delayed?  
5. Should the system store full document content or only chunks?  
6. Should deleted files remove chunks or mark them inactive?  
7. Should hybrid search use RuVector-native functions or application-level rank fusion?  
8. Should feedback affect ranking automatically or remain analysis-only at first?  
9. Should the bot be Telegram, Slack, or both?

---

## **17\. Recommended Implementation Order**

The recommended order is:

1. RuVector setup spike.  
2. Markdown ingestion.  
3. Vector search CLI.  
4. Retrieval evaluation.  
5. RAG with citations.  
6. Hybrid search.  
7. Feedback reporting.  
8. Local API.  
9. Telegram bot.  
10. Advanced RuVector experiments.

Do not start with the bot.  
Do not start with an autonomous agent.  
Do not start with self-learning.  
Start with measurable retrieval.

---

## **18\. Definition of Done for Learning Project**

The project is done when the user can:

* Run the system locally.  
* Ingest a folder of technical documents.  
* Ask natural-language questions.  
* See the retrieved chunks.  
* Get cited answers.  
* Run retrieval evaluation.  
* Compare vector vs keyword vs hybrid search.  
* Capture feedback.  
* Explain where RuVector fits in the architecture.  
* Explain when retrieval fails and how to improve it.

The real deliverable is not just the tool. The real deliverable is the ability to reason clearly about vector search systems.
