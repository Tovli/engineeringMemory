use std::fs;

#[test]
fn publish_workflow_bumps_versions_before_discovery_on_main() {
    let workflow_path = format!(
        "{}/.github/workflows/publish-crates.yml",
        env!("CARGO_MANIFEST_DIR")
    );
    let workflow = fs::read_to_string(&workflow_path).expect("publish workflow should be readable");

    assert!(
        workflow.contains("      - main"),
        "publish workflow should run on pushes to main"
    );

    let bump_step = workflow
        .find("- name: Auto-bump crate versions")
        .expect("publish workflow should auto-bump crate versions");
    let discover_step = workflow
        .find("- name: Discover unpublished versions")
        .expect("publish workflow should discover unpublished versions");

    assert!(
        bump_step < discover_step,
        "crate versions must be auto-bumped before crates.io discovery"
    );

    for expected in [
        "$env:GITHUB_RUN_NUMBER",
        "manifest_path",
        "Set-Content",
        "cargo update --workspace",
    ] {
        assert!(
            workflow.contains(expected),
            "publish workflow should contain `{expected}`"
        );
    }
}
