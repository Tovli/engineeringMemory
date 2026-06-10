//! Local keyword-index record shape and scoring helpers (M5 / ADR-0009).
//! This is infrastructure support shared by the ingestion writer and retrieval reader.

use std::collections::{BTreeMap, HashMap, HashSet};

use redb::TableDefinition;
use serde::{Deserialize, Serialize};

use crate::ingestion::domain::Chunk;

pub const KEYWORD_CHUNKS: TableDefinition<&str, &str> = TableDefinition::new("keyword_chunks");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordIndexedChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub source_path: String,
    pub heading_path: Vec<String>,
    pub content: String,
    pub preview: String,
    pub metadata: BTreeMap<String, String>,
}

impl KeywordIndexedChunk {
    pub fn from_chunk(chunk: &Chunk) -> Self {
        Self {
            chunk_id: chunk.id.clone(),
            document_id: chunk.document_id.clone(),
            source_path: chunk
                .metadata
                .get("source_path")
                .cloned()
                .unwrap_or_default(),
            heading_path: chunk.heading_path.clone(),
            content: chunk.content.clone(),
            preview: chunk.preview.clone(),
            metadata: chunk.metadata.clone(),
        }
    }
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            buf.push(ch.to_ascii_lowercase());
        } else if !buf.is_empty() {
            terms.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        terms.push(buf);
    }
    terms
}

pub fn document_frequencies<'a>(
    query_terms: impl IntoIterator<Item = &'a String>,
    documents: &[Vec<String>],
) -> HashMap<String, usize> {
    let wanted: HashSet<&String> = query_terms.into_iter().collect();
    let mut freqs = HashMap::new();
    for doc_terms in documents {
        let unique: HashSet<&String> = doc_terms.iter().filter(|t| wanted.contains(*t)).collect();
        for term in unique {
            *freqs.entry(term.clone()).or_insert(0) += 1;
        }
    }
    freqs
}

pub fn bm25_score(
    query_terms: &[String],
    doc_terms: &[String],
    doc_freqs: &HashMap<String, usize>,
    doc_count: usize,
    avg_doc_len: f32,
) -> f32 {
    if query_terms.is_empty() || doc_terms.is_empty() || doc_count == 0 || avg_doc_len <= 0.0 {
        return 0.0;
    }
    let k1 = 1.2;
    let b = 0.75;
    let doc_len = doc_terms.len() as f32;
    let mut term_counts: HashMap<&String, usize> = HashMap::new();
    for term in doc_terms {
        *term_counts.entry(term).or_insert(0) += 1;
    }

    let mut score = 0.0;
    let mut seen = HashSet::new();
    for term in query_terms {
        if !seen.insert(term) {
            continue;
        }
        let tf = *term_counts.get(term).unwrap_or(&0) as f32;
        if tf == 0.0 {
            continue;
        }
        let df = *doc_freqs.get(term).unwrap_or(&0) as f32;
        if df == 0.0 {
            continue;
        }
        let idf = (1.0 + ((doc_count as f32 - df + 0.5) / (df + 0.5))).ln();
        let denom = tf + k1 * (1.0 - b + b * (doc_len / avg_doc_len));
        score += idf * ((tf * (k1 + 1.0)) / denom);
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_handles_technical_terms_case_insensitively() {
        assert_eq!(
            tokenize("zipDeploy 403 npm-ci"),
            vec!["zipdeploy", "403", "npm", "ci"]
        );
    }

    #[test]
    fn bm25_scores_matching_terms_above_misses() {
        let q = tokenize("zipDeploy 403");
        let docs = vec![
            tokenize("Azure Function zipDeploy 403"),
            tokenize("architecture boundaries"),
        ];
        let dfs = document_frequencies(q.iter(), &docs);
        let avg = docs.iter().map(|d| d.len() as f32).sum::<f32>() / docs.len() as f32;
        assert!(bm25_score(&q, &docs[0], &dfs, docs.len(), avg) > 0.0);
        assert_eq!(bm25_score(&q, &docs[1], &dfs, docs.len(), avg), 0.0);
    }
}
