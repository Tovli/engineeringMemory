//! Image memories remain searchable and their originals survive a store backup.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageOutputFormat, RgbaImage};
use tovli::ingestion::durable::{add, add_image, get_image, ImageInput, Input};
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::image_memory::links::{create_for_store, LinkInput};

fn workspace() -> PathBuf {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("tovli-image-workflow-{}-{timestamp}", std::process::id()))
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let dest = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn search(workspace: &Path, text: &str, mode: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(workspace)
        .args(["search", text, "--mock", "--mode", mode, "--project", "payments"])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn cli_finds_image_text_reports_missing_and_backup_restores_original() {
    let workspace = workspace();
    let store = workspace.join(".tovli");
    let backup = workspace.join("backup");
    std::fs::create_dir_all(&store).unwrap();
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::new(2, 3))
        .write_to(&mut png, ImageOutputFormat::Png).unwrap();
    let original = png.into_inner();
    let input: ImageInput = serde_json::from_value(serde_json::json!({
        "source_key": "queue-diagram", "mime_type": "image/png",
        "description": "Diagram of the payment retry queue",
        "ocr_text": "INVOICE-FAILURE-431",
        "project": "payments", "tags": ["incident"]
    })).unwrap();
    let embedder = MockEmbedder::new(384);
    let saved = add_image(&store, &embedder, input.clone(), original.clone()).unwrap();
    let asset_id = saved["asset_id"].as_str().unwrap();
    let asset_hash = saved["original_hash"].as_str().unwrap();
    let note: Input = serde_json::from_value(serde_json::json!({
        "source_key": "queue-note",
        "content": "Payment incident ROLLBACK-349 involved a retry queue.",
        "project": "payments", "tags": ["incident"]
    })).unwrap();
    let note_receipt = add(&store, &embedder, note).unwrap();
    create_for_store(&store, &LinkInput {
        document_id: note_receipt.document_id.clone(),
        image_document_id: saved["document_id"].as_str().unwrap().into(),
        chunk_id: None,
        caption: Some("Retry architecture".into()),
    }).unwrap();
    let mut other_input = input;
    other_input.source_key = "other-diagram".into();
    other_input.project = Some("other".into());
    let other = add_image(&store, &embedder, other_input, original.clone()).unwrap();
    assert_eq!(other["asset_id"], asset_id);
    create_for_store(&store, &LinkInput {
        document_id: note_receipt.document_id,
        image_document_id: other["document_id"].as_str().unwrap().into(),
        chunk_id: None,
        caption: Some("Unrelated project".into()),
    }).unwrap();
    let note_result = search(&workspace, "ROLLBACK-349", "keyword");
    assert!(note_result.contains("Retry architecture"), "{note_result}");
    assert!(note_result.contains(asset_id), "{note_result}");
    assert!(!note_result.contains("Unrelated project"), "{note_result}");
    for query in ["payment retry", "INVOICE-FAILURE-431"] {
        for mode in ["keyword", "hybrid"] {
            let result = search(&workspace, query, mode);
            assert!(result.contains("mcp://local/queue-diagram"), "{result}");
            assert!(result.contains(asset_id), "{result}");
            assert!(result.contains("(present)"), "{result}");
        }
    }
    copy_tree(&store, &backup);
    let restored_workspace = workspace.join("restored");
    let restored_store = restored_workspace.join(".tovli");
    copy_tree(&backup, &restored_store);
    assert_eq!(get_image(&restored_store, asset_id).unwrap().1, original);
    assert!(search(&restored_workspace, "INVOICE-FAILURE-431", "keyword")
        .contains("mcp://local/queue-diagram"));
    let original_path = store.join("assets").join(format!("{asset_hash}.png"));
    std::fs::remove_file(&original_path).unwrap();
    let result = search(&workspace, "INVOICE-FAILURE-431", "keyword");
    assert!(result.contains("(asset_missing)"), "{result}");
    assert!(get_image(&store, asset_id).is_err());
    std::fs::copy(backup.join("assets").join(format!("{asset_hash}.png")), &original_path).unwrap();
    assert_eq!(get_image(&store, asset_id).unwrap().1, original);
    std::fs::remove_dir_all(workspace).unwrap();
}
