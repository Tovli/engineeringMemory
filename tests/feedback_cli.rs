//! M6 CLI smoke test: search persists retrieval evidence, feedback records against it, and
//! feedback-report can export the append-only feedback log.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("tovli-m6-cli-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write_doc(&self, name: &str, body: &str) {
        let docs = self.root.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join(name), body).unwrap();
    }
}

fn run(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "command {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn parse_field(stdout: &str, name: &str) -> String {
    stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .find_map(|pair| (pair[0] == name).then(|| pair[1].to_string()))
        })
        .unwrap_or_else(|| panic!("missing {name} in stdout:\n{stdout}"))
}

fn parse_first_chunk(stdout: &str) -> String {
    let start = stdout.find("[chunk_").unwrap_or_else(|| {
        panic!("missing chunk id in stdout:\n{stdout}");
    });
    let end = stdout[start + 1..]
        .find(']')
        .map(|i| start + 1 + i)
        .expect("chunk id closing bracket");
    stdout[start + 1..end].to_string()
}

fn read_run_log(root: &Path) -> Vec<serde_json::Value> {
    let body = std::fs::read_to_string(root.join(".tovli").join("retrieval-runs.jsonl")).unwrap();
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn cli_records_feedback_for_a_search_result_and_exports_it() {
    let project = TempProject::new();
    let body = "layering rules and component boundaries are important";
    project.write_doc("arch.md", &format!("# Architecture\n\n{body}\n"));

    run(&project.root, &["ingest", "docs", "--mock"]);
    let search = run(
        &project.root,
        &["search", body, "--mock", "--top-k", "3", "--mode", "vector"],
    );
    let query_id = parse_field(&search, "query-id:");
    let run_id = parse_field(&search, "run-id:");
    let chunk_id = parse_first_chunk(&search);

    let feedback = run(
        &project.root,
        &[
            "feedback",
            "--query-id",
            &query_id,
            "--run-id",
            &run_id,
            "--good",
            &chunk_id,
            "--note",
            "useful retrieval",
        ],
    );
    assert!(feedback.contains("recorded feedback"));

    let export_path = project.root.join("feedback.json");
    let report = run(
        &project.root,
        &["feedback-report", "--export", export_path.to_str().unwrap()],
    );
    assert!(report.contains("total feedback : 1"));
    assert!(
        report.contains("observations    : 1"),
        "report should print observation details:\n{report}"
    );
    assert!(
        report.contains(&format!("run={run_id} rank=1 score=1.0000 mode=vector")),
        "report should print run/rank/score/mode details:\n{report}"
    );

    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(export_path).unwrap()).unwrap();
    assert_eq!(exported[0]["queryId"], query_id);
    assert_eq!(exported[0]["retrievalRunId"], run_id);
    assert_eq!(exported[0]["chunkId"], chunk_id);
}

#[test]
fn cli_lists_no_good_queries_in_feedback_report() {
    let project = TempProject::new();
    let body = "zipDeploy 403 release failed";
    project.write_doc("deploy.md", &format!("# Deploy\n\n{body}\n"));

    run(&project.root, &["ingest", "docs", "--mock"]);
    let search = run(&project.root, &["search", body, "--mock", "--top-k", "3"]);
    let query_id = parse_field(&search, "query-id:");
    let run_id = parse_field(&search, "run-id:");
    let chunk_id = parse_first_chunk(&search);

    run(
        &project.root,
        &[
            "feedback",
            "--query-id",
            &query_id,
            "--run-id",
            &run_id,
            "--bad",
            &chunk_id,
        ],
    );

    let report = run(&project.root, &["feedback-report"]);
    assert!(report.contains("no-good queries : 1"));
    assert!(
        report.contains(&format!("- {query_id} feedback=1 good=0")),
        "report should list no-good query details:\n{report}"
    );
    assert!(
        report.contains(&query_id),
        "report should list the no-good query id:\n{report}"
    );
    assert!(
        report.contains(body),
        "report should list the no-good question text:\n{report}"
    );
}

#[test]
fn search_on_empty_index_prints_ids_and_persists_empty_run_evidence() {
    let project = TempProject::new();

    let search = run(&project.root, &["search", "anything", "--mock"]);
    let query_id = parse_field(&search, "query-id:");
    let run_id = parse_field(&search, "run-id:");
    assert!(search.contains("index is empty"));

    let runs = read_run_log(&project.root);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["queryId"], query_id);
    assert_eq!(runs[0]["retrievalRunId"], run_id);
    assert_eq!(runs[0]["results"].as_array().unwrap().len(), 0);
}

#[test]
fn ask_on_empty_index_prints_ids_and_persists_empty_run_evidence() {
    let project = TempProject::new();

    let ask = run(&project.root, &["ask", "anything", "--mock", "--no-llm"]);
    let query_id = parse_field(&ask, "query-id:");
    let run_id = parse_field(&ask, "run-id:");
    assert!(ask.contains("index is empty"));

    let runs = read_run_log(&project.root);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["queryId"], query_id);
    assert_eq!(runs[0]["retrievalRunId"], run_id);
    assert_eq!(runs[0]["results"].as_array().unwrap().len(), 0);
}

#[test]
fn cli_generates_distinct_run_ids_for_back_to_back_searches() {
    let project = TempProject::new();
    let body = "layering rules and component boundaries are important";
    project.write_doc("arch.md", &format!("# Architecture\n\n{body}\n"));

    run(&project.root, &["ingest", "docs", "--mock"]);
    let first = run(&project.root, &["search", body, "--mock"]);
    let second = run(&project.root, &["search", body, "--mock"]);

    assert_ne!(
        parse_field(&first, "query-id:"),
        parse_field(&second, "query-id:")
    );
    assert_ne!(
        parse_field(&first, "run-id:"),
        parse_field(&second, "run-id:")
    );
}

#[test]
fn cli_rejects_mixed_feedback_batch_without_partial_persistence() {
    let project = TempProject::new();
    let body = "layering rules and component boundaries are important";
    project.write_doc("arch.md", &format!("# Architecture\n\n{body}\n"));

    run(&project.root, &["ingest", "docs", "--mock"]);
    let search = run(&project.root, &["search", body, "--mock"]);
    let query_id = parse_field(&search, "query-id:");
    let run_id = parse_field(&search, "run-id:");
    let chunk_id = parse_first_chunk(&search);

    let output = Command::new(env!("CARGO_BIN_EXE_tovli"))
        .current_dir(&project.root)
        .args([
            "feedback",
            "--query-id",
            &query_id,
            "--run-id",
            &run_id,
            "--good",
            &chunk_id,
            "--bad",
            "chunk_not_displayed",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("was not displayed"));

    let export_path = project.root.join("feedback.json");
    run(
        &project.root,
        &["feedback-report", "--export", export_path.to_str().unwrap()],
    );
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(export_path).unwrap()).unwrap();
    assert_eq!(exported.as_array().unwrap().len(), 0);
}
