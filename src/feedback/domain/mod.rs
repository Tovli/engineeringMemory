//! Feedback domain types. Pure data and report shapes, no storage dependencies.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ingestion::domain::{ChunkId, DocumentId};
use crate::retrieval::domain::{RetrievalRun, SearchMode};

pub type FeedbackId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    Good,
    Bad,
}

impl std::fmt::Display for FeedbackRating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedbackRating::Good => write!(f, "good"),
            FeedbackRating::Bad => write!(f, "bad"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackItem {
    pub id: FeedbackId,
    pub query_id: String,
    pub retrieval_run_id: String,
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub rating: FeedbackRating,
    pub note: Option<String>,
    pub search_mode: SearchMode,
    pub rank: usize,
    pub score: f32,
    pub source_path: String,
    pub question_text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedChunkEvidence {
    pub rank: usize,
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub source_path: String,
    pub score: f32,
    pub preview: String,
    pub heading_path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRunEvidence {
    pub retrieval_run_id: String,
    pub query_id: String,
    pub question_text: String,
    pub search_mode: SearchMode,
    pub top_k: usize,
    pub created_at: String,
    pub results: Vec<RetrievedChunkEvidence>,
}

impl RetrievalRunEvidence {
    pub fn from_run(run: &RetrievalRun, query_id: &str) -> Self {
        Self {
            retrieval_run_id: run.id.clone(),
            query_id: query_id.to_string(),
            question_text: run.query.text.clone(),
            search_mode: run.search_mode,
            top_k: run.top_k,
            created_at: run.completed_at.clone(),
            results: run
                .results
                .iter()
                .map(|r| RetrievedChunkEvidence {
                    rank: r.rank,
                    chunk_id: r.chunk_id.clone(),
                    document_id: r.document_id.clone(),
                    source_path: r.source_path.clone(),
                    score: r.score,
                    preview: r.preview.clone(),
                    heading_path: r.heading_path.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FeedbackQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub rating: Option<FeedbackRating>,
}

impl FeedbackQuery {
    pub fn matches(&self, item: &FeedbackItem) -> bool {
        if let Some(from) = &self.from {
            if item.created_at < *from {
                return false;
            }
        }
        if let Some(to) = &self.to {
            if item.created_at > *to {
                return false;
            }
        }
        if let Some(rating) = self.rating {
            if item.rating != rating {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReport {
    pub id: String,
    pub generated_at: String,
    pub total_feedback: usize,
    pub observations: Vec<FeedbackObservation>,
    pub problematic_queries: Vec<ProblematicQuery>,
    pub frequently_downvoted_chunks: Vec<DownvotedChunk>,
    pub queries_with_no_good_result: Vec<NoGoodResultQuery>,
    pub candidates_for_rechunking: Vec<RechunkingCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackObservation {
    pub feedback_id: FeedbackId,
    pub query_id: String,
    pub retrieval_run_id: String,
    pub chunk_id: ChunkId,
    pub rating: FeedbackRating,
    pub search_mode: SearchMode,
    pub rank: usize,
    pub score: f32,
    pub source_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblematicQuery {
    pub query_id: String,
    pub question_text: String,
    pub search_modes: Vec<SearchMode>,
    pub bad_count: usize,
    pub good_count: usize,
    pub bad_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownvotedChunk {
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub source_path: String,
    pub bad_count: usize,
    pub distinct_query_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoGoodResultQuery {
    pub query_id: String,
    pub question_text: String,
    pub total_feedback: usize,
    pub good_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RechunkingCandidate {
    pub document_id: DocumentId,
    pub source_path: String,
    pub downvoted_chunk_count: usize,
    pub reason: String,
}

pub(crate) fn sorted_modes(modes: BTreeMap<SearchModeKey, SearchMode>) -> Vec<SearchMode> {
    modes.into_values().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SearchModeKey {
    Vector,
    Keyword,
    Hybrid,
}

impl From<SearchMode> for SearchModeKey {
    fn from(value: SearchMode) -> Self {
        match value {
            SearchMode::Vector => SearchModeKey::Vector,
            SearchMode::Keyword => SearchModeKey::Keyword,
            SearchMode::Hybrid => SearchModeKey::Hybrid,
        }
    }
}
