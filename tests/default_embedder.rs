#[test]
fn default_cargo_features_enable_the_real_onnx_embedder() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read Cargo.toml");

    let features = manifest
        .split_once("[features]")
        .map(|(_, features)| features)
        .expect("Cargo.toml has a [features] section");

    assert!(
        features
            .lines()
            .any(|line| line.trim() == r#"default = ["onnx"]"#),
        "default users should get OnnxEmbedder; add `default = [\"onnx\"]` under [features]"
    );
}
