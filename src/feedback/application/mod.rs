//! Feedback application services.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::feedback::domain::{
    sorted_modes, DownvotedChunk, FeedbackItem, FeedbackObservation, FeedbackRating,
    FeedbackReport, NoGoodResultQuery, ProblematicQuery, RechunkingCandidate, RetrievalRunEvidence,
    SearchModeKey,
};
use crate::feedback::ports::{FeedbackRepository, RetrievalRunEvidenceStore};
use crate::ingestion::domain::ChunkId;

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRunIds {
    pub query_id: String,
    pub retrieval_run_id: String,
}

pub fn next_query_run_ids(timestamp_millis: i64) -> QueryRunIds {
    let seq = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    QueryRunIds {
        query_id: format!("qry_{timestamp_millis}_{pid}_{seq}"),
        retrieval_run_id: format!("rrun_{timestamp_millis}_{pid}_{seq}"),
    }
}

#[derive(Debug, Clone)]
pub struct RecordFeedbackInput {
    pub feedback_id: String,
    pub query_id: String,
    pub retrieval_run_id: String,
    pub chunk_id: ChunkId,
    pub rating: FeedbackRating,
    pub note: Option<String>,
    pub created_at: String,
}

pub struct FeedbackService<'a, F: FeedbackRepository, R: RetrievalRunEvidenceStore> {
    pub feedback: &'a F,
    pub runs: &'a R,
}

impl<F: FeedbackRepository, R: RetrievalRunEvidenceStore> FeedbackService<'_, F, R> {
    pub fn record(&self, input: RecordFeedbackInput) -> anyhow::Result<FeedbackItem> {
        let items = self.build_items(&[input])?;
        let item = items
            .into_iter()
            .next()
            .expect("one input produces one feedback item");
        self.feedback.save(&item)?;
        Ok(item)
    }

    pub fn record_many(
        &self,
        inputs: Vec<RecordFeedbackInput>,
    ) -> anyhow::Result<Vec<FeedbackItem>> {
        let items = self.build_items(&inputs)?;
        for item in &items {
            self.feedback.save(item)?;
        }
        Ok(items)
    }

    fn build_items(&self, inputs: &[RecordFeedbackInput]) -> anyhow::Result<Vec<FeedbackItem>> {
        let mut batch_ids = HashSet::new();
        for input in inputs {
            if !batch_ids.insert(input.feedback_id.as_str()) {
                anyhow::bail!(
                    "feedback id '{}' appears more than once in this batch",
                    input.feedback_id
                );
            }
            if self.feedback.find_by_id(&input.feedback_id)?.is_some() {
                anyhow::bail!("feedback id '{}' already exists", input.feedback_id);
            }
        }

        let mut runs: HashMap<String, RetrievalRunEvidence> = HashMap::new();
        let mut items = Vec::with_capacity(inputs.len());
        for input in inputs {
            let run = if let Some(run) = runs.get(&input.retrieval_run_id) {
                run.clone()
            } else {
                let run = self
                    .runs
                    .find_by_run_id(&input.retrieval_run_id)?
                    .ok_or_else(|| {
                        anyhow::anyhow!("retrieval run '{}' was not found", input.retrieval_run_id)
                    })?;
                runs.insert(input.retrieval_run_id.clone(), run.clone());
                run
            };
            items.push(build_item(input, run)?);
        }
        Ok(items)
    }
}

fn build_item(
    input: &RecordFeedbackInput,
    run: RetrievalRunEvidence,
) -> anyhow::Result<FeedbackItem> {
    if run.query_id != input.query_id {
        anyhow::bail!(
            "query id '{}' does not match retrieval run '{}' (expected '{}')",
            input.query_id,
            input.retrieval_run_id,
            run.query_id
        );
    }

    let shown = run
        .results
        .iter()
        .find(|r| r.chunk_id == input.chunk_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "chunk '{}' was not displayed in retrieval run '{}'",
                input.chunk_id,
                input.retrieval_run_id
            )
        })?;

    Ok(FeedbackItem {
        id: input.feedback_id.clone(),
        query_id: input.query_id.clone(),
        retrieval_run_id: input.retrieval_run_id.clone(),
        chunk_id: shown.chunk_id.clone(),
        document_id: shown.document_id.clone(),
        rating: input.rating,
        note: input.note.clone(),
        search_mode: run.search_mode,
        rank: shown.rank,
        score: shown.score,
        source_path: shown.source_path.clone(),
        question_text: run.question_text,
        created_at: input.created_at.clone(),
    })
}

pub struct FeedbackReportService<'a, F: FeedbackRepository> {
    pub feedback: &'a F,
}

impl<F: FeedbackRepository> FeedbackReportService<'_, F> {
    pub fn generate(&self, id: &str, generated_at: &str) -> anyhow::Result<FeedbackReport> {
        let items = self.feedback.find_all(None)?;
        Ok(build_report(id, generated_at, &items, 2))
    }
}

fn build_report(
    id: &str,
    generated_at: &str,
    items: &[FeedbackItem],
    rechunk_threshold: usize,
) -> FeedbackReport {
    #[derive(Default)]
    struct QueryAgg {
        question_text: String,
        modes: BTreeMap<SearchModeKey, crate::retrieval::domain::SearchMode>,
        bad: usize,
        good: usize,
    }
    #[derive(Default)]
    struct ChunkAgg {
        document_id: String,
        source_path: String,
        bad: usize,
        query_ids: BTreeSet<String>,
    }
    #[derive(Default)]
    struct DocAgg {
        source_path: String,
        bad_chunks: BTreeSet<String>,
    }

    let mut by_query: HashMap<String, QueryAgg> = HashMap::new();
    let mut by_chunk: HashMap<String, ChunkAgg> = HashMap::new();
    let mut by_doc: HashMap<String, DocAgg> = HashMap::new();
    let mut observations = Vec::with_capacity(items.len());

    for item in items {
        observations.push(FeedbackObservation {
            feedback_id: item.id.clone(),
            query_id: item.query_id.clone(),
            retrieval_run_id: item.retrieval_run_id.clone(),
            chunk_id: item.chunk_id.clone(),
            rating: item.rating,
            search_mode: item.search_mode,
            rank: item.rank,
            score: item.score,
            source_path: item.source_path.clone(),
            created_at: item.created_at.clone(),
        });

        let q = by_query.entry(item.query_id.clone()).or_default();
        if q.question_text.is_empty() {
            q.question_text = item.question_text.clone();
        }
        q.modes.insert(item.search_mode.into(), item.search_mode);
        match item.rating {
            FeedbackRating::Good => q.good += 1,
            FeedbackRating::Bad => q.bad += 1,
        }

        if item.rating == FeedbackRating::Bad {
            let c = by_chunk.entry(item.chunk_id.clone()).or_default();
            c.document_id = item.document_id.clone();
            c.source_path = item.source_path.clone();
            c.bad += 1;
            c.query_ids.insert(item.query_id.clone());

            let d = by_doc.entry(item.document_id.clone()).or_default();
            d.source_path = item.source_path.clone();
            d.bad_chunks.insert(item.chunk_id.clone());
        }
    }

    let mut problematic_queries: Vec<ProblematicQuery> = by_query
        .iter()
        .filter_map(|(query_id, agg)| {
            let total = agg.bad + agg.good;
            if total == 0 || agg.bad == 0 {
                return None;
            }
            Some(ProblematicQuery {
                query_id: query_id.clone(),
                question_text: agg.question_text.clone(),
                search_modes: sorted_modes(agg.modes.clone()),
                bad_count: agg.bad,
                good_count: agg.good,
                bad_ratio: agg.bad as f64 / total as f64,
            })
        })
        .collect();
    problematic_queries.sort_by(|a, b| {
        b.bad_ratio
            .partial_cmp(&a.bad_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.bad_count.cmp(&a.bad_count))
            .then_with(|| a.query_id.cmp(&b.query_id))
    });

    let mut frequently_downvoted_chunks: Vec<DownvotedChunk> = by_chunk
        .into_iter()
        .map(|(chunk_id, agg)| DownvotedChunk {
            chunk_id,
            document_id: agg.document_id,
            source_path: agg.source_path,
            bad_count: agg.bad,
            distinct_query_count: agg.query_ids.len(),
        })
        .collect();
    frequently_downvoted_chunks.sort_by(|a, b| {
        b.bad_count
            .cmp(&a.bad_count)
            .then_with(|| b.distinct_query_count.cmp(&a.distinct_query_count))
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });

    let mut queries_with_no_good_result: Vec<NoGoodResultQuery> = by_query
        .into_iter()
        .filter_map(|(query_id, agg)| {
            let total = agg.bad + agg.good;
            if total == 0 || agg.good > 0 {
                return None;
            }
            Some(NoGoodResultQuery {
                query_id,
                question_text: agg.question_text,
                total_feedback: total,
                good_count: agg.good,
            })
        })
        .collect();
    queries_with_no_good_result.sort_by(|a, b| {
        b.total_feedback
            .cmp(&a.total_feedback)
            .then_with(|| a.query_id.cmp(&b.query_id))
    });

    let mut candidates_for_rechunking: Vec<RechunkingCandidate> = by_doc
        .into_iter()
        .filter_map(|(document_id, agg)| {
            let count = agg.bad_chunks.len();
            if count < rechunk_threshold {
                return None;
            }
            Some(RechunkingCandidate {
                document_id,
                source_path: agg.source_path,
                downvoted_chunk_count: count,
                reason: format!("{count} distinct chunks in this document received bad feedback"),
            })
        })
        .collect();
    candidates_for_rechunking.sort_by(|a, b| {
        b.downvoted_chunk_count
            .cmp(&a.downvoted_chunk_count)
            .then_with(|| a.document_id.cmp(&b.document_id))
    });

    FeedbackReport {
        id: id.to_string(),
        generated_at: generated_at.to_string(),
        total_feedback: items.len(),
        observations,
        problematic_queries,
        frequently_downvoted_chunks,
        queries_with_no_good_result,
        candidates_for_rechunking,
    }
}
