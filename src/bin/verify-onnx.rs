//! M1 task #13 — verify the Local ONNX embedding path works on this machine.
//!
//! Loads all-MiniLM-L6-v2 via ruvector-core's OnnxEmbedding (ort + tokenizers, downloads
//! the model from HuggingFace on first run), embeds three words, and checks that the
//! embeddings are *semantic*: "dog" is closer to "cat" than to "car". If this passes, the
//! MSVC + ort + onnxruntime + MiniLM stack is proven and M1 can build on it.
//!
//! Run: cargo run --features onnx --bin verify-onnx

use ruvector_core::{EmbeddingProvider, OnnxEmbedding};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

fn main() -> anyhow::Result<()> {
    println!("== ONNX embedding verification (MiniLM via ort) ==");
    let model = "sentence-transformers/all-MiniLM-L6-v2";
    // Load from pre-downloaded local files (offline, C2) — avoids the hf-hub 0.3
    // RelativeUrlWithoutBase bug in from_pretrained.
    let model_path = std::path::PathBuf::from("models/all-MiniLM-L6-v2/model.onnx");
    let tokenizer_path = std::path::PathBuf::from("models/all-MiniLM-L6-v2/tokenizer.json");
    println!("loading {model} from local files...");

    let started = std::time::Instant::now();
    let embedder = OnnxEmbedding::from_files(&model_path, &tokenizer_path, model)
        .map_err(|e| anyhow::anyhow!("model load failed: {e}"))?;
    println!("model loaded in {:?}", started.elapsed());

    let words = ["dog", "cat", "car"];
    let t0 = std::time::Instant::now();
    let embs: Vec<Vec<f32>> = words
        .iter()
        .map(|w| embedder.embed(w).map_err(|e| anyhow::anyhow!("embed failed: {e}")))
        .collect::<anyhow::Result<_>>()?;
    let dim = embs[0].len();
    println!("embedded {} words, dim={dim}, in {:?}\n", words.len(), t0.elapsed());

    let dog_cat = cosine(&embs[0], &embs[1]);
    let dog_car = cosine(&embs[0], &embs[2]);
    println!("cosine(dog, cat) = {dog_cat:.4}");
    println!("cosine(dog, car) = {dog_car:.4}");

    let checks = [
        ("dim == 384", dim == 384),
        ("semantic: dog~cat > dog~car", dog_cat > dog_car),
    ];
    println!("\nchecks:");
    let mut ok = true;
    for (name, pass) in checks {
        println!("  [{}] {name}", if pass { "PASS" } else { "FAIL" });
        ok &= pass;
    }

    if ok {
        println!("\nRESULT: PASS — Local ONNX embeddings work (MSVC + ort + MiniLM verified).");
        Ok(())
    } else {
        eprintln!("\nRESULT: FAIL — see failing checks.");
        std::process::exit(1);
    }
}
