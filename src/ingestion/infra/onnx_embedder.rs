//! OnnxEmbedder — Embedder port backed by ruvector-core's local ONNX/MiniLM model.
//! Uses `from_files` with local/cache files (`from_pretrained` is broken in hf-hub 0.3).
//! Compiled when the `onnx` Cargo feature is enabled.

use std::path::{Path, PathBuf};

use ruvector_core::OnnxEmbedding;

use crate::ingestion::domain::EmbeddingModelVersion;
use crate::ingestion::ports::Embedder;

const MINILM_MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
const MINILM_DIMENSION: usize = 384;
const MINILM_ENV_DIR: &str = "TOVLI_MINILM_DIR";
const MINILM_LOCAL_DIR: &str = "models/all-MiniLM-L6-v2";
const MINILM_MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
const MINILM_TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

#[derive(Debug, Clone)]
struct MiniLmFiles {
    model: PathBuf,
    tokenizer: PathBuf,
}

fn minilm_files_in(dir: &Path) -> MiniLmFiles {
    MiniLmFiles {
        model: dir.join("model.onnx"),
        tokenizer: dir.join("tokenizer.json"),
    }
}

fn missing_minilm_files(files: &MiniLmFiles) -> Vec<String> {
    let mut missing = Vec::new();
    if !files.model.is_file() {
        missing.push(files.model.display().to_string());
    }
    if !files.tokenizer.is_file() {
        missing.push(files.tokenizer.display().to_string());
    }
    missing
}

fn require_minilm_dir(dir: &Path, source: &str) -> anyhow::Result<MiniLmFiles> {
    let files = minilm_files_in(dir);
    let missing = missing_minilm_files(&files);
    if missing.is_empty() {
        Ok(files)
    } else {
        anyhow::bail!(
            "MiniLM model directory from {source} is incomplete.\nmissing:\n  - {}\n\n\
             Expected both model.onnx and tokenizer.json. Set {MINILM_ENV_DIR} to a complete \
             directory or run with --mock for deterministic non-semantic embeddings.",
            missing.join("\n  - ")
        )
    }
}

fn default_minilm_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".tovli/cache"))
        .join("tovli")
        .join("models")
        .join("all-MiniLM-L6-v2")
}

fn download_file(url: &str, dest: &Path) -> anyhow::Result<()> {
    if dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }

    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid destination {}", dest.display()))?;
    std::fs::create_dir_all(parent)?;

    let filename = dest
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid destination filename {}", dest.display()))?;
    let tmp = dest.with_file_name(format!("{filename}.download"));
    let _ = std::fs::remove_file(&tmp);

    let response = ureq::get(url)
        .set("User-Agent", concat!("tovli/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| anyhow::anyhow!("download failed for {url}: {e}"))?;

    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&tmp)?;
    std::io::copy(&mut reader, &mut file)?;
    drop(file);

    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn download_minilm_files(dir: &Path) -> anyhow::Result<MiniLmFiles> {
    let files = minilm_files_in(dir);
    download_file(MINILM_MODEL_URL, &files.model)?;
    download_file(MINILM_TOKENIZER_URL, &files.tokenizer)?;
    require_minilm_dir(dir, "tovli cache")
}

fn resolve_minilm_files() -> anyhow::Result<MiniLmFiles> {
    if let Some(dir) = std::env::var_os(MINILM_ENV_DIR)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        return require_minilm_dir(&dir, MINILM_ENV_DIR);
    }

    let local = PathBuf::from(MINILM_LOCAL_DIR);
    if local.exists() {
        return require_minilm_dir(&local, MINILM_LOCAL_DIR);
    }

    let cache = default_minilm_cache_dir();
    if missing_minilm_files(&minilm_files_in(&cache)).is_empty() {
        return Ok(minilm_files_in(&cache));
    }

    download_minilm_files(&cache).map_err(|e| {
        anyhow::anyhow!(
            "MiniLM model files were not found locally and automatic download failed: {e}\n\n\
             Looked for:\n  - {MINILM_ENV_DIR}=<directory containing model.onnx and tokenizer.json>\n  - \
             {MINILM_LOCAL_DIR}/model.onnx and tokenizer.json\n  - {}\n\n\
             You can also run with --mock to use deterministic non-semantic embeddings.",
            cache.display()
        )
    })
}

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
        Self::open_paths(
            &PathBuf::from(model_path),
            &PathBuf::from(tokenizer_path),
            name,
            dimension,
        )
    }

    pub fn open_paths(
        model_path: &Path,
        tokenizer_path: &Path,
        name: &str,
        dimension: usize,
    ) -> anyhow::Result<Self> {
        let inner = OnnxEmbedding::from_files(
            &model_path.to_path_buf(),
            &tokenizer_path.to_path_buf(),
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

    /// Load the default all-MiniLM-L6-v2 model from env/local files or the tovli cache.
    pub fn open_minilm() -> anyhow::Result<Self> {
        let files = resolve_minilm_files()?;
        Self::open_paths(
            &files.model,
            &files.tokenizer,
            MINILM_MODEL_ID,
            MINILM_DIMENSION,
        )
    }
}

impl Embedder for OnnxEmbedder {
    fn model_version(&self) -> &EmbeddingModelVersion {
        &self.model
    }

    fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner
            .embed_batch(texts)
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}
