//! RedbKeywordSearch — read-only KeywordSearchPort over the local keyword index.

use redb::{Database, ReadableTable};

use crate::lexical_index::{
    bm25_score, document_frequencies, tokenize, KeywordIndexedChunk, KEYWORD_CHUNKS,
};
use crate::retrieval::ports::{KeywordSearchPort, RawKeywordSearchResult};

pub struct RedbKeywordSearch {
    db: Option<Database>,
}

impl RedbKeywordSearch {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        match Database::open(path) {
            Ok(db) => Ok(Self { db: Some(db) }),
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(e)))
                if e.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Self { db: None })
            }
            Err(e) => Err(e.into()),
        }
    }

    fn all(&self) -> anyhow::Result<Vec<KeywordIndexedChunk>> {
        let Some(db) = &self.db else {
            return Ok(vec![]);
        };
        let rtxn = db.begin_read()?;
        let table = match rtxn.open_table(KEYWORD_CHUNKS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for row in table.iter()? {
            let (_, v) = row?;
            out.push(serde_json::from_str(v.value())?);
        }
        Ok(out)
    }
}

impl KeywordSearchPort for RedbKeywordSearch {
    fn keyword_search(&self, query: &str, k: usize) -> anyhow::Result<Vec<RawKeywordSearchResult>> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || k == 0 {
            return Ok(vec![]);
        }
        let chunks = self.all()?;
        if chunks.is_empty() {
            return Ok(vec![]);
        }
        let tokenized: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(&c.content)).collect();
        let doc_freqs = document_frequencies(query_terms.iter(), &tokenized);
        let avg_doc_len =
            tokenized.iter().map(|d| d.len() as f32).sum::<f32>() / tokenized.len() as f32;

        let mut scored: Vec<(KeywordIndexedChunk, f32)> = chunks
            .into_iter()
            .zip(tokenized.iter())
            .filter_map(|(chunk, terms)| {
                let score = bm25_score(
                    &query_terms,
                    terms,
                    &doc_freqs,
                    tokenized.len(),
                    avg_doc_len,
                );
                (score > 0.0).then_some((chunk, score))
            })
            .collect();

        scored.sort_by(|(a, ascore), (b, bscore)| {
            bscore
                .total_cmp(ascore)
                .then_with(|| a.source_path.cmp(&b.source_path))
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });

        Ok(scored
            .into_iter()
            .take(k)
            .map(|(chunk, raw_score)| RawKeywordSearchResult {
                chunk_id: chunk.chunk_id,
                document_id: chunk.document_id,
                source_path: chunk.source_path,
                raw_score,
                preview: chunk.preview,
                heading_path: chunk.heading_path,
                metadata: chunk.metadata,
            })
            .collect())
    }
}
