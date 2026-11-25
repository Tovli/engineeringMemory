//! Shared processing for filesystem and logical documents.
use std::collections::BTreeMap;

use super::chunking::ChunkingService;
use super::domain::{Chunk, DocumentId};
use super::embedding::EmbeddingService;
use super::ports::{ChunkWithEmbedding, Embedder, KeywordIndexPort, ParsedDoc, VectorStorePort};

pub fn prepare_chunks(
    chunking: &ChunkingService,
    parsed: &ParsedDoc,
    extension: &str,
    document_id: &str,
    source_path: &str,
    embedder: &dyn Embedder,
    extra: &BTreeMap<String, String>,
) -> anyhow::Result<(Vec<Chunk>, Vec<Vec<f32>>)> {
    let model = embedder.model_version();
    let mut chunks = chunking.chunk(parsed, extension, document_id);
    for chunk in &mut chunks {
        chunk
            .metadata
            .insert("source_path".into(), source_path.into());
        chunk.metadata.insert(
            "char_length".into(),
            chunk.content.chars().count().to_string(),
        );
        chunk
            .metadata
            .insert("embedding_model".into(), model.name.clone());
        chunk
            .metadata
            .insert("embedding_dimension".into(), model.dimension.to_string());
        if let Some(title) = &parsed.title {
            chunk.metadata.insert("title".into(), title.clone());
        }
        chunk.metadata.extend(extra.clone());
    }
    let vectors = EmbeddingService::new(embedder).embed_chunks(&chunks)?;
    Ok((chunks, vectors))
}

pub fn persist_chunks(
    store: &dyn VectorStorePort,
    keyword: Option<&dyn KeywordIndexPort>,
    chunks: &[Chunk],
    vectors: Vec<Vec<f32>>,
) -> anyhow::Result<()> {
    if chunks.len() != vectors.len() {
        anyhow::bail!("chunk and vector counts differ");
    }
    let items: Vec<ChunkWithEmbedding> = chunks
        .iter()
        .zip(vectors)
        .map(|(chunk, vector)| ChunkWithEmbedding { chunk, vector })
        .collect();
    store.upsert_chunks(&items)?;
    if let Some(index) = keyword {
        index.upsert_chunks(&chunks.iter().collect::<Vec<_>>())?;
    }
    Ok(())
}

pub fn remove_document_chunks(
    store: &dyn VectorStorePort,
    keyword: Option<&dyn KeywordIndexPort>,
    document_id: &DocumentId,
) -> anyhow::Result<()> {
    store.delete_by_document(document_id)?;
    if let Some(index) = keyword {
        index.delete_by_document(document_id)?;
    }
    Ok(())
}
