//! OnnxEmbedder — Embedder port backed by ruvector-core's local ONNX/MiniLM model.
//! Uses `from_files` with a locally cached model (from_pretrained is broken in hf-hub 0.3).
//! Compiled when the `onnx` Cargo feature is enabled.

use std::path::PathBuf;

use ruvector_core::OnnxEmbedding;

use crate::ingestion::domain::EmbeddingModelVersion;
use crate::ingestion::ports::Embedder;

pub struct OnnxEmbedder {
    inner: OnnxEmbedding,
    model: EmbeddingModelVersion,
}

impl OnnxEmbedder {
    pub fn open(
        model_path: &str,
        tokenizer_path: &str,
        name: &str,
        dimension: usize,
    ) -> anyhow::Result<Self> {
        let inner = OnnxEmbedding::from_files(
            &PathBuf::from(model_path),
            &PathBuf::from(tokenizer_path),
            name,
        )
        .map_err(|e| anyhow::anyhow!("ONNX model load failed: {e}"))?;
        Ok(Self {
            inner,
            model: EmbeddingModelVersion {
                name: name.to_string(),
                dimension,
                created_at: "2026-06-02T00:00:00Z".to_string(),
            },
        })
    }

    /// Load the default all-MiniLM-L6-v2 model from `models/` (dim 384).
    pub fn open_minilm() -> anyhow::Result<Self> {
        Self::open(
            "models/all-MiniLM-L6-v2/model.onnx",
            "models/all-MiniLM-L6-v2/tokenizer.json",
            "sentence-transformers/all-MiniLM-L6-v2",
            384,
        )
    }
}

impl Embedder for OnnxEmbedder {
    fn model_version(&self) -> &EmbeddingModelVersion {
        &self.model
    }

    fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner.embed_batch(texts).map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}
