//! EmbeddingService — embeds chunk text via the `Embedder` port and enforces the
//! dimension invariant (PRD FR-EMB-001, AC-8): every vector matches the model dimension.

use crate::ingestion::domain::{Chunk, EmbeddingModelVersion};
use crate::ingestion::ports::Embedder;

pub struct EmbeddingService<'a> {
    embedder: &'a dyn Embedder,
}

impl<'a> EmbeddingService<'a> {
    pub fn new(embedder: &'a dyn Embedder) -> Self {
        Self { embedder }
    }

    pub fn model_version(&self) -> &EmbeddingModelVersion {
        self.embedder.model_version()
    }

    /// Embed each chunk's content; guarantees `vec.len() == model dimension`.
    pub fn embed_chunks(&self, chunks: &[Chunk]) -> anyhow::Result<Vec<Vec<f32>>> {
        if chunks.is_empty() {
            return Ok(vec![]);
        }
        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let vectors = self.embedder.embed_batch(&texts)?;
        let dim = self.embedder.model_version().dimension;
        if vectors.len() != chunks.len() {
            anyhow::bail!(
                "embedder returned {} vectors for {} chunks",
                vectors.len(),
                chunks.len()
            );
        }
        for v in &vectors {
            if v.len() != dim {
                anyhow::bail!("embedder returned dim {} != model dim {dim}", v.len());
            }
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Embedder stub that returns whatever vectors it is told to — lets us drive the
    /// count/dimension invariants without a real model.
    struct StubEmbedder {
        model: EmbeddingModelVersion,
        returns: Vec<Vec<f32>>,
    }
    impl StubEmbedder {
        fn new(dim: usize, returns: Vec<Vec<f32>>) -> Self {
            Self {
                model: EmbeddingModelVersion {
                    name: "stub".into(),
                    dimension: dim,
                    created_at: "1970-01-01T00:00:00Z".into(),
                },
                returns,
            }
        }
    }
    impl Embedder for StubEmbedder {
        fn model_version(&self) -> &EmbeddingModelVersion {
            &self.model
        }
        fn embed_batch(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(self.returns.clone())
        }
    }

    fn chunk(id: &str) -> Chunk {
        Chunk {
            id: id.into(),
            document_id: "d".into(),
            chunk_index: 0,
            heading_path: vec![],
            content: format!("text {id}"),
            preview: "p".into(),
            content_hash: "h".into(),
            token_count: 1,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_chunks_returns_empty_and_skips_the_embedder() {
        // Returns the wrong-dimension vector that WOULD fail the invariant if the embedder
        // were called — proving the empty short-circuit runs first.
        let emb = StubEmbedder::new(8, vec![vec![9.9; 99]]);
        let out = EmbeddingService::new(&emb).embed_chunks(&[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn happy_path_returns_one_vector_per_chunk_at_model_dim() {
        let emb = StubEmbedder::new(4, vec![vec![0.1; 4], vec![0.2; 4]]);
        let out = EmbeddingService::new(&emb).embed_chunks(&[chunk("c1"), chunk("c2")]).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|v| v.len() == 4));
    }

    #[test]
    fn count_mismatch_bails() {
        // Embedder returns fewer vectors than chunks (FR-EMB-001).
        let emb = StubEmbedder::new(4, vec![vec![0.1; 4]]);
        let err = EmbeddingService::new(&emb)
            .embed_chunks(&[chunk("c1"), chunk("c2")])
            .unwrap_err();
        assert!(format!("{err:#}").contains("vectors for"), "got: {err:#}");
    }

    #[test]
    fn dimension_mismatch_bails() {
        // Vector dim (3) != model dim (4) — the AC-8 invariant must reject it.
        let emb = StubEmbedder::new(4, vec![vec![0.1; 3]]);
        let err = EmbeddingService::new(&emb).embed_chunks(&[chunk("c1")]).unwrap_err();
        assert!(format!("{err:#}").contains("!= model dim"), "got: {err:#}");
    }
}
