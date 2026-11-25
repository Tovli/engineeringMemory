#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn exchange(stdin: &mut impl Write, stdout: &mut impl BufRead, message: Value) -> Value {
    writeln!(stdin, "{message}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn stdio_tools_save_retry_and_query() {
    let workspace = std::env::temp_dir().join(format!(
        "tovli-mcp-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let folder = workspace.join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("queue.md"),
        "# Worker queue\nA CLI note about queue retries.",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tovli-mcp"))
        .arg("--store-dir")
        .arg(workspace.join(".").join(".tovli"))
        .arg("--mock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let initialized = exchange(
        &mut input,
        &mut output,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18","capabilities":{},
                      "clientInfo":{"name":"tovli-test","version":"1"}}
        }),
    );
    assert!(initialized.get("result").is_some(), "{initialized}");
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let list = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    );
    assert!(list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "tovli_add_document"));

    let args = json!({"source_key":"incident-1","content":"# Incident\nQueue retry caused duplicates.",
                      "metadata":{"severity":2,"reviewed":true}});
    let request = |id, arguments: Value| {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
            "name":"tovli_add_document","arguments":arguments
        }})
    };
    let created = exchange(&mut input, &mut output, request(3, args.clone()));
    assert_eq!(
        created["result"]["structuredContent"]["status"], "created",
        "{created}"
    );

    let cli_search = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(&workspace)
        .args(["search", "queue retries", "--mock"])
        .output()
        .unwrap();
    assert!(
        cli_search.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_search.stderr)
    );
    assert!(String::from_utf8_lossy(&cli_search.stdout).contains("mcp://local/incident-1"));

    let retried = exchange(&mut input, &mut output, request(4, args.clone()));
    assert_eq!(
        retried["result"]["structuredContent"]["status"], "unchanged",
        "{retried}"
    );
    assert_eq!(
        created["result"]["structuredContent"]["document_id"],
        retried["result"]["structuredContent"]["document_id"]
    );

    let query = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
            "name":"tovli_query","arguments":{"source_key":"incident-1"}
        }}),
    );
    assert_eq!(
        query["result"]["structuredContent"]["results"][0]["metadata"]["severity"], 2,
        "{query}"
    );
    let conflict = exchange(
        &mut input,
        &mut output,
        request(
            6,
            json!({
                "source_key":"incident-1","content":"different"
            }),
        ),
    );
    assert_eq!(conflict["result"]["isError"], true, "{conflict}");
    assert_eq!(
        conflict["result"]["structuredContent"]["code"],
        "source_conflict"
    );
    let invalid = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{
            "name":"tovli_query","arguments":{}
        }}),
    );
    assert_eq!(invalid["error"]["code"], -32602, "{invalid}");
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 3))
        .write_to(&mut png, image::ImageOutputFormat::Png)
        .unwrap();
    use base64::Engine;
    let image_data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let cli_ingest = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(&workspace)
        .args(["ingest", "notes", "--mock"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let image = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{
            "name":"tovli_add_image","arguments":{"source_key":"diagram-1","image_base64":image_data,"mime_type":"image/png","description":"Queue retries diagram"}
        }}),
    );
    let cli_ingest = cli_ingest.wait_with_output().unwrap();
    assert!(
        cli_ingest.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_ingest.stderr)
    );
    assert_eq!(
        image["result"]["structuredContent"]["status"], "created",
        "{image}"
    );
    let image_id = image["result"]["structuredContent"]["document_id"]
        .as_str()
        .unwrap();
    let asset_id = image["result"]["structuredContent"]["asset_id"]
        .as_str()
        .unwrap();
    let document_id = created["result"]["structuredContent"]["document_id"]
        .as_str()
        .unwrap();
    let link = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{
            "name":"tovli_link_image","arguments":{"document_id":document_id,"image_document_id":image_id,"caption":"Retry loop"}
        }}),
    );
    assert_eq!(
        link["result"]["structuredContent"]["status"], "created",
        "{link}"
    );
    let listing = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{
            "name":"tovli_list_image_links","arguments":{"image_document_id":image_id}
        }}),
    );
    assert_eq!(
        listing["result"]["structuredContent"]["results"][0]["caption"], "Retry loop",
        "{listing}"
    );
    let second = exchange(
        &mut input,
        &mut output,
        request(
            12,
            json!({
                "source_key":"incident-2","content":"# Another incident\nA second payment queue incident.",
                "project":"payments","tags":["reviewed"]
            }),
        ),
    );
    assert_eq!(
        second["result"]["structuredContent"]["status"], "created",
        "{second}"
    );
    let second_id = second["result"]["structuredContent"]["document_id"]
        .as_str()
        .unwrap();
    let second_link = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":13,"method":"tools/call","params":{
            "name":"tovli_link_image","arguments":{"document_id":second_id,"image_document_id":image_id}
        }}),
    );
    assert_eq!(
        second_link["result"]["structuredContent"]["status"], "created",
        "{second_link}"
    );
    let filtered = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":14,"method":"tools/call","params":{
            "name":"tovli_list_image_links","arguments":{"image_document_id":image_id,"project":"payments","tags":["reviewed"]}
        }}),
    );
    let filtered_results = filtered["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap();
    assert_eq!(filtered_results.len(), 1, "{filtered}");
    assert_eq!(filtered_results[0]["document_id"], second_id);
    let first_page = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":15,"method":"tools/call","params":{
            "name":"tovli_list_image_links","arguments":{"image_document_id":image_id,"limit":1}
        }}),
    );
    let page = &first_page["result"]["structuredContent"];
    assert_eq!(page["results"].as_array().unwrap().len(), 1, "{first_page}");
    let cursor = page["next_cursor"].as_str().unwrap();
    let next_page = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":16,"method":"tools/call","params":{
            "name":"tovli_list_image_links","arguments":{"image_document_id":image_id,"limit":1,"cursor":cursor}
        }}),
    );
    let next = &next_page["result"]["structuredContent"];
    assert_eq!(next["results"].as_array().unwrap().len(), 1, "{next_page}");
    assert_ne!(page["results"][0]["link_id"], next["results"][0]["link_id"]);
    assert!(next["next_cursor"].is_null());
    let wrong_cursor = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":17,"method":"tools/call","params":{
            "name":"tovli_list_image_links","arguments":{"document_id":document_id,"cursor":cursor}
        }}),
    );
    assert_eq!(
        wrong_cursor["result"]["structuredContent"]["code"], "invalid_cursor",
        "{wrong_cursor}"
    );
    let oversized = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":18,"method":"tools/call","params":{
            "name":"tovli_query","arguments":{"source_key":"incident-1"},
            "_meta":{"padding":"x".repeat(1024 * 1024)}
        }}),
    );
    assert_eq!(oversized["error"]["code"], -32602, "{oversized}");
    let fetched = exchange(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{
            "name":"tovli_get_image","arguments":{"asset_id":asset_id}
        }}),
    );
    assert_eq!(
        fetched["result"]["content"][1]["data"], image_data,
        "{fetched}"
    );

    let search = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(&workspace)
        .args(["search", "queue retries", "--mock"])
        .output()
        .unwrap();
    assert!(
        search.status.success(),
        "{}",
        String::from_utf8_lossy(&search.stderr)
    );
    let results = String::from_utf8_lossy(&search.stdout);
    for source in [
        "mcp://local/incident-1",
        "queue.md",
        "mcp://local/diagram-1",
    ] {
        assert!(results.contains(source), "missing {source} in {results}");
    }
    // Hold the store lock so the first request occupies the single worker.
    // The cancelled second request must be discarded before it reaches storage.
    let lock = tovli::store_lock::StoreLock::acquire(
        &workspace.join(".tovli"),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":19,"method":"tools/call","params":{
            "name":"tovli_query","arguments":{"source_key":"incident-1"}
        }})
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    writeln!(
        input,
        "{}",
        request(
            20,
            json!({
                "source_key":"cancelled-request","content":"This queued document is cancelled."
            })
        )
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/cancelled",
        "params":{"requestId":20,"reason":"test cancellation"}})
    )
    .unwrap();
    input.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    drop(lock);
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":21,"method":"tools/call","params":{
            "name":"tovli_query","arguments":{"source_key":"cancelled-request"}
        }})
    )
    .unwrap();
    input.flush().unwrap();
    let mut cancelled_result = None;
    while cancelled_result.is_none() {
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["id"] == 21 {
            cancelled_result = Some(message);
        }
    }
    assert_eq!(
        cancelled_result.unwrap()["result"]["structuredContent"]["results"],
        json!([])
    );
    drop(input);
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(workspace).unwrap();
}
