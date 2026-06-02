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
