//! SearchService — the application core for the Retrieval context.
//! Pipeline: validate → guard model → embed → over-fetch → join docs → filter → rank → explain.
//! Generic over the read ports via trait objects (mirrors M1's IngestionOrchestrator), so it is
//! fully unit-testable with in-memory fakes + MockEmbedder (no ONNX, no disk).

use std::collections::BTreeSet;
use std::time::Instant;

use crate::ingestion::domain::{DocumentId, EmbeddingModelVersion};
use crate::ingestion::ports::Embedder;
use crate::retrieval::application::filters::passes;
use crate::retrieval::application::scoring::{fetch_k, similarity_from_distance, SIMILARITY_THRESHOLD};
use crate::retrieval::domain::errors::RetrievalError;
use crate::retrieval::domain::explain::{ExplainPayload, ExplainResultDetail};
use crate::retrieval::domain::query::Query;
use crate::retrieval::domain::retrieval_result::RetrievalResult;
use crate::retrieval::domain::retrieval_run::{RetrievalRun, RunReason};
use crate::retrieval::ports::{DocumentLookupPort, VectorSearchPort};

pub struct SearchService<'a> {
    pub embedder: &'a dyn Embedder,
    pub store: &'a dyn VectorSearchPort,
    pub lookup: &'a dyn DocumentLookupPort,
}

fn models_match(a: &EmbeddingModelVersion, b: &EmbeddingModelVersion) -> bool {
    // created_at intentionally ignored — only name + dimension define compatibility.
    a.name == b.name && a.dimension == b.dimension
}

impl SearchService<'_> {
    /// Execute one search. `run_id` and `now` (RFC3339) are injected for determinism (like M1).
    pub fn search(
        &self,
        query: &Query,
        explain: bool,
        run_id: &str,
        now: &str,
    ) -> anyhow::Result<RetrievalRun> {
        let t0 = Instant::now();

        // 1. Guard rails ---------------------------------------------------- (R7, E1/E2)
        if query.text.trim().is_empty() {
            return Err(RetrievalError::EmptyQuery.into());
        }
        if query.top_k == 0 {
            return Err(RetrievalError::InvalidTopK.into());
        }

        // 2. Index presence + model compatibility --------------------------- (R6, AC-7/AC-8, E3/E8)
        let indexed = self.lookup.indexed_model_version()?;
        let Some(indexed) = indexed else {
            return Ok(empty_run(query, run_id, now, RunReason::IndexEmpty, t0));
        };
        if !models_match(&indexed, &query.embedding_model) {
            return Err(RetrievalError::EmbeddingModelMismatch {
                indexed,
                query: query.embedding_model.clone(),
            }
            .into());
        }

        // 3. Embed the query ------------------------------------------------- (R1, C4)
        let qvec = {
            let mut v = self.embedder.embed_batch(&[query.text.as_str()])?;
            v.pop().ok_or_else(|| anyhow::anyhow!("embedder returned no vector"))?
        };
        anyhow::ensure!(
            qvec.len() == indexed.dimension,
            "query vector dim {} != index dim {}",
            qvec.len(),
            indexed.dimension
        );

        // 4. Over-fetch candidates ------------------------------------------ (E5, ADR-0003)
        let filters_set = !query.filters.is_empty();
        let raw = self.store.vector_search(&qvec, fetch_k(query.top_k, filters_set))?;
        let candidate_count = raw.len();

        // 5. Resolve owning-document metadata for the join ------------------ (E6, E9, ADR-0002)
        let doc_ids: Vec<DocumentId> = raw
            .iter()
            .map(|r| r.document_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let docs = self.lookup.find_many(&doc_ids)?;

        // 6. Filter (preserving knn order), trim to top_k, normalize score --- (R3, AC-2/3/4, E4/E7/E9)
        let mut results: Vec<RetrievalResult> = Vec::new();
        let mut details: Vec<ExplainResultDetail> = Vec::new();
        for (knn_pos, r) in raw.iter().enumerate() {
            if results.len() == query.top_k {
                break;
            }
            let Some(doc) = docs.get(&r.document_id) else {
                continue; // orphaned candidate (doc record gone) — skip (E9)
            };
            if !passes(&query.filters, r, doc) {
                continue;
            }
            let rank = results.len() + 1;
            let score = similarity_from_distance(r.distance);
            if explain {
                details.push(ExplainResultDetail {
                    chunk_id: r.chunk_id.clone(),
                    rank,
                    vector_score: Some(score),
                    keyword_score: None,
                    fused_score: score,
                    eligibility_reason: format!(
                        "knn #{} of {} candidates; passed {} filter(s)",
                        knn_pos + 1,
                        candidate_count,
                        if filters_set { "active" } else { "no" }
                    ),
                });
            }
            results.push(RetrievalResult {
                rank,
                chunk_id: r.chunk_id.clone(),
                document_id: r.document_id.clone(),
                source_path: r.source_path.clone(),
                score,
                preview: r.preview.clone(),
                heading_path: r.heading_path.clone(),
                metadata: r.metadata.clone(),
            });
        }

        let below_threshold_count = results.iter().filter(|r| r.score < SIMILARITY_THRESHOLD).count();

        // 7. Explain payload ------------------------------------------------- (R5, AC-6)
        let explain_payload = explain.then(|| ExplainPayload {
            query_embedding_provider: indexed.name.clone(),
            query_embedding_dimension: indexed.dimension,
            search_mode: query.mode.to_string(),
            filters_applied: query.filters.clone(),
            ranking_method: "cosine".to_string(),
            result_details: details,
        });

        // 8. Assemble immutable run ----------------------------------------- (R8)
        Ok(RetrievalRun {
            id: run_id.to_string(),
            query: query.clone(),
            results,
            search_mode: query.mode,
            top_k: query.top_k,
            latency_ms: t0.elapsed().as_millis(),
            below_threshold_count,
            reason: RunReason::Ok,
            explain: explain_payload,
            completed_at: now.to_string(),
        })
    }
}

fn empty_run(query: &Query, run_id: &str, now: &str, reason: RunReason, t0: Instant) -> RetrievalRun {
    RetrievalRun {
        id: run_id.to_string(),
        query: query.clone(),
        results: Vec::new(),
        search_mode: query.mode,
        top_k: query.top_k,
        latency_ms: t0.elapsed().as_millis(),
        below_threshold_count: 0,
        reason,
        explain: None,
        completed_at: now.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    use crate::ingestion::domain::DocumentStatus;
    use crate::ingestion::infra::mock_embedder::MockEmbedder;
    use crate::retrieval::domain::query::{MetadataFilter, SearchMode};
    use crate::retrieval::ports::{DocMeta, RawSearchResult};

    const DIM: usize = 8;
    const NOW: &str = "2026-06-03T00:00:00Z";

    fn model(name: &str, dim: usize) -> EmbeddingModelVersion {
        EmbeddingModelVersion { name: name.into(), dimension: dim, created_at: NOW.into() }
    }

    // ---- fakes ----
    struct FakeStore {
        hits: Vec<RawSearchResult>,
    }
    impl VectorSearchPort for FakeStore {
        fn vector_search(&self, _q: &[f32], k: usize) -> anyhow::Result<Vec<RawSearchResult>> {
            Ok(self.hits.iter().take(k).cloned().collect())
        }
    }

    struct FakeLookup {
        model: Option<EmbeddingModelVersion>,
        docs: HashMap<DocumentId, DocMeta>,
    }
    impl DocumentLookupPort for FakeLookup {
        fn find_many(&self, ids: &[DocumentId]) -> anyhow::Result<HashMap<DocumentId, DocMeta>> {
            Ok(ids.iter().filter_map(|id| self.docs.get(id).map(|d| (id.clone(), d.clone()))).collect())
        }
        fn indexed_model_version(&self) -> anyhow::Result<Option<EmbeddingModelVersion>> {
            Ok(self.model.clone())
        }
    }

    fn hit(chunk: &str, doc: &str, source: &str, distance: f32) -> RawSearchResult {
        RawSearchResult {
            chunk_id: chunk.into(),
            document_id: doc.into(),
            source_path: source.into(),
            distance,
            preview: format!("preview of {chunk}"),
            heading_path: vec!["H".into()],
            metadata: BTreeMap::new(),
        }
    }
    fn doc(project: Option<&str>, tags: &[&str], source: &str, status: DocumentStatus) -> DocMeta {
        DocMeta {
            project: project.map(String::from),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            source_path: source.into(),
            status,
        }
    }

    fn query(filters: MetadataFilter, top_k: usize) -> Query {
        Query {
            text: "architecture layering rules".into(),
            mode: SearchMode::Vector,
            filters,
            top_k,
            embedding_model: model("mock-deterministic", DIM),
        }
    }

    fn service<'a>(emb: &'a MockEmbedder, store: &'a FakeStore, lookup: &'a FakeLookup) -> SearchService<'a> {
        SearchService { embedder: emb, store, lookup }
    }

    #[test]
    fn returns_ranked_results_best_first() {
        // AC-1 + AC-2: ranked, score non-increasing, source + score present.
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore {
            hits: vec![
                hit("c1", "d1", "docs/a.md", 0.10),
                hit("c2", "d2", "docs/b.md", 0.25),
                hit("c3", "d3", "docs/c.md", 0.60),
            ],
        };
        let mut docs = HashMap::new();
        for (id, src) in [("d1", "docs/a.md"), ("d2", "docs/b.md"), ("d3", "docs/c.md")] {
            docs.insert(id.to_string(), doc(None, &[], src, DocumentStatus::Active));
        }
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };

        let run = service(&emb, &store, &lookup).search(&query(MetadataFilter::default(), 8), false, "rrun_1", NOW).unwrap();

        assert_eq!(run.reason, RunReason::Ok);
        assert_eq!(run.results.len(), 3);
        assert_eq!(run.results[0].chunk_id, "c1");
        assert_eq!(run.results[0].rank, 1);
        assert_eq!(run.results[0].source_path, "docs/a.md");
        // scores non-increasing (AC-2)
        assert!(run.results[0].score >= run.results[1].score);
        assert!(run.results[1].score >= run.results[2].score);
        // distance 0.10 → similarity 0.90
        assert!((run.results[0].score - 0.90).abs() < 1e-5);
    }

    #[test]
    fn trims_to_top_k() {
        // AC-3
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore {
            hits: (0..10).map(|i| hit(&format!("c{i}"), &format!("d{i}"), "s.md", i as f32 * 0.05)).collect(),
        };
        let docs = (0..10).map(|i| (format!("d{i}"), doc(None, &[], "s.md", DocumentStatus::Active))).collect();
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };
        let run = service(&emb, &store, &lookup).search(&query(MetadataFilter::default(), 3), false, "r", NOW).unwrap();
        assert_eq!(run.results.len(), 3);
    }

    #[test]
    fn empty_index_yields_index_empty_reason_not_error() {
        // AC-8
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![] };
        let lookup = FakeLookup { model: None, docs: HashMap::new() };
        let run = service(&emb, &store, &lookup).search(&query(MetadataFilter::default(), 8), false, "r", NOW).unwrap();
        assert_eq!(run.reason, RunReason::IndexEmpty);
        assert!(run.results.is_empty());
    }

    #[test]
    fn model_mismatch_is_an_error_and_runs_nothing() {
        // AC-7
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![hit("c1", "d1", "a.md", 0.1)] };
        let lookup = FakeLookup { model: Some(model("openai-3-small", 1536)), docs: HashMap::new() };
        let err = service(&emb, &store, &lookup)
            .search(&query(MetadataFilter::default(), 8), false, "r", NOW)
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("embedding model mismatch"), "got: {msg}");
        assert!(msg.contains("openai-3-small") && msg.contains("mock-deterministic"));
    }

    #[test]
    fn filters_by_project_tag_source() {
        // AC-4 — over-fetch then app-side filter (ADR-0002/0003)
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore {
            hits: vec![
                hit("c1", "d1", "docs/a.md", 0.10), // flexid, [arch]
                hit("c2", "d2", "docs/b.md", 0.20), // other,  [arch]
                hit("c3", "d3", "docs/a.md", 0.30), // flexid, [ops]
                hit("c4", "d4", "docs/a.md", 0.40), // flexid, [arch] ✓
            ],
        };
        let mut docs = HashMap::new();
        docs.insert("d1".into(), doc(Some("flexid"), &["arch"], "docs/a.md", DocumentStatus::Active));
        docs.insert("d2".into(), doc(Some("other"), &["arch"], "docs/b.md", DocumentStatus::Active));
        docs.insert("d3".into(), doc(Some("flexid"), &["ops"], "docs/a.md", DocumentStatus::Active));
        docs.insert("d4".into(), doc(Some("flexid"), &["arch"], "docs/a.md", DocumentStatus::Active));
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };

        let f = MetadataFilter {
            project: Some("flexid".into()),
            tags: vec!["arch".into()],
            source: Some("docs/a.md".into()),
        };
        let run = service(&emb, &store, &lookup).search(&query(f, 8), false, "r", NOW).unwrap();
        // only c1 and c4 satisfy project=flexid AND tag=arch AND source=docs/a.md
        let ids: Vec<&str> = run.results.iter().map(|r| r.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c1", "c4"]);
    }

    #[test]
    fn no_results_after_filter_is_ok_run() {
        // AC-5 / E4
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![hit("c1", "d1", "a.md", 0.1)] };
        let mut docs = HashMap::new();
        docs.insert("d1".into(), doc(Some("other"), &[], "a.md", DocumentStatus::Active));
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };
        let f = MetadataFilter { project: Some("flexid".into()), ..Default::default() };
        let run = service(&emb, &store, &lookup).search(&query(f, 8), false, "r", NOW).unwrap();
        assert_eq!(run.reason, RunReason::Ok);
        assert!(run.results.is_empty());
    }

    #[test]
    fn deleted_document_is_excluded() {
        // E9
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![hit("c1", "d1", "a.md", 0.1), hit("c2", "d2", "b.md", 0.2)] };
        let mut docs = HashMap::new();
        docs.insert("d1".into(), doc(None, &[], "a.md", DocumentStatus::Deleted));
        docs.insert("d2".into(), doc(None, &[], "b.md", DocumentStatus::Active));
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };
        let run = service(&emb, &store, &lookup).search(&query(MetadataFilter::default(), 8), false, "r", NOW).unwrap();
        let ids: Vec<&str> = run.results.iter().map(|r| r.chunk_id.as_str()).collect();
        assert_eq!(ids, vec!["c2"]);
    }

    #[test]
    fn explain_payload_is_populated_when_requested() {
        // AC-6
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![hit("c1", "d1", "a.md", 0.1)] };
        let mut docs = HashMap::new();
        docs.insert("d1".into(), doc(None, &[], "a.md", DocumentStatus::Active));
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs };
        let run = service(&emb, &store, &lookup).search(&query(MetadataFilter::default(), 8), true, "r", NOW).unwrap();
        let ex = run.explain.expect("explain payload present");
        assert_eq!(ex.ranking_method, "cosine");
        assert_eq!(ex.query_embedding_dimension, DIM);
        assert_eq!(ex.result_details.len(), 1);
        assert_eq!(ex.result_details[0].rank, 1);
        assert!(ex.result_details[0].eligibility_reason.contains("knn #1"));
    }

    #[test]
    fn empty_query_and_zero_topk_are_rejected() {
        // E1 / E2
        let emb = MockEmbedder::new(DIM);
        let store = FakeStore { hits: vec![] };
        let lookup = FakeLookup { model: Some(model("mock-deterministic", DIM)), docs: HashMap::new() };
        let svc = service(&emb, &store, &lookup);

        let mut q = query(MetadataFilter::default(), 8);
        q.text = "   ".into();
        assert!(svc.search(&q, false, "r", NOW).unwrap_err().to_string().contains("must not be empty"));

        let q0 = query(MetadataFilter::default(), 0);
        assert!(svc.search(&q0, false, "r", NOW).unwrap_err().to_string().contains("top-k"));
    }
}
