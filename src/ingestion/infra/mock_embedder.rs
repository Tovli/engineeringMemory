//! MockEmbedder — deterministic, offline embedder for tests (PRD FR-EMB-001).
//! Not semantic; maps text → a stable vector derived from its blake3 digest.

use crate::ingestion::domain::EmbeddingModelVersion;
use crate::ingestion::ports::Embedder;

pub struct MockEmbedder {
    model: EmbeddingModelVersion,
}

impl MockEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self {
            model: EmbeddingModelVersion {
                name: "mock-deterministic".to_string(),
                dimension,
                created_at: "1970-01-01T00:00:00Z".to_string(),
            },
        }
    }
}

impl Embedder for MockEmbedder {
    fn model_version(&self) -> &EmbeddingModelVersion {
        &self.model
    }

    fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let dim = self.model.dimension;
        Ok(texts
            .iter()
            .map(|t| {
                let digest = blake3::hash(t.as_bytes());
                let bytes = digest.as_bytes();
                (0..dim).map(|i| bytes[i % 32] as f32 / 255.0).collect()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_correct_dimension() {
        let e = MockEmbedder::new(8);
        let a = e.embed_batch(&["hello"]).unwrap();
        let b = e.embed_batch(&["hello"]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[0].len(), 8);
        assert_ne!(e.embed_batch(&["world"]).unwrap()[0], a[0]);
    }
}
