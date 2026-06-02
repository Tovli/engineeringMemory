# tovli — Domain-Driven Design Model

This directory contains the complete Domain-Driven Design model for the tovli
RuVector-Based Technical Memory Assistant.

> All definitions and constraints here are derived from
> [`docs/prd.md`](../prd.md). When the PRD and this model conflict, the PRD
> is authoritative.

---

## How the Files Relate

```
docs/ddd/
├── README.md                  ← this file; index and orientation
├── context-map.md             ← bounded contexts and their relationships
├── ubiquitous-language.md     ← domain glossary
├── aggregates.md              ← consolidated aggregate inventory with invariants
├── domain-events.md           ← event catalog with payload sketches
└── contexts/
    ├── ingestion.md           ← Ingestion bounded context
    ├── retrieval.md           ← Retrieval & Search bounded context
    ├── answer-generation.md   ← RAG Answer Generation bounded context
    ├── evaluation.md          ← Evaluation bounded context
    ├── feedback.md            ← Feedback bounded context
    └── shared-kernel.md       ← Shared Kernel (identifiers and value objects)
```

---

## Reading Order

1. **Start with** `ubiquitous-language.md` to learn the domain vocabulary.
2. **Then read** `context-map.md` to understand the bounded contexts and how
   they relate to each other.
3. **Then read** the context files under `contexts/` in pipeline order:
   `ingestion` → `retrieval` → `answer-generation` → `evaluation` → `feedback`.
   Read `shared-kernel` alongside the others.
4. **Then read** `aggregates.md` for a consolidated invariant reference.
5. **Then read** `domain-events.md` for the cross-context event flows.

---

## Bounded Context Summary

| Context             | Core Responsibility                                              |
|---------------------|------------------------------------------------------------------|
| Ingestion           | Parse, chunk, embed, and store documents idempotently            |
| Retrieval           | Execute vector/keyword/hybrid search with metadata filtering     |
| Answer Generation   | Build cited answers from retrieved chunks using an LLM           |
| Evaluation          | Measure retrieval quality with Hit@K and MRR metrics             |
| Feedback            | Capture and report user ratings on retrieved chunks              |
| Shared Kernel       | Common identifiers and value objects used across all contexts    |

---

## Key Design Principles (from PRD §7)

- **Retrieval Before Generation**: retrieval quality must be proven before
  the LLM answer layer is added. This is modelled as an ordering invariant
  between the Evaluation and Answer Generation contexts.
- **Mandatory Citations**: every Answer aggregate must carry at least one
  Citation value object — an answer without citations is invalid by invariant.
- **Observability as a Feature**: every aggregate that crosses a context
  boundary publishes a domain event with a full observability payload
  (scores, latency, filters, model version).
- **Local-First**: RuVector/Postgres and the LLM provider are external systems
  behind Anti-Corruption Layers. The domain model never imports their types
  directly.
- **Embedding Model Versioning**: an Embedding value object is bound to a
  (modelName, dimension) pair. Mixing embeddings with different dimensions
  in one index is an invariant violation.
