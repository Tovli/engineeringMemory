use std::fs;

#[test]
fn publish_workflow_uses_committed_tagged_versions() {
    let workflow = read_workspace_file(".github/workflows/publish-crates.yml");

    assert!(
        workflow.contains("    tags:"),
        "publish workflow should run on tag pushes"
    );
    assert!(
        workflow.contains("      - 'v*'"),
        "publish workflow should run for release tags"
    );
    assert!(
        !workflow.contains("    branches:"),
        "publish workflow should not publish on branch pushes"
    );
    assert!(
        !workflow.contains("Auto-bump crate versions"),
        "publish workflow should not mutate crate versions"
    );
    assert!(
        !workflow.contains("GITHUB_RUN_NUMBER"),
        "publish workflow should not derive versions from CI run numbers"
    );
    assert!(
        !workflow.contains("--allow-dirty"),
        "publish workflow should keep Cargo's dirty-tree protection"
    );
    assert!(
        !workflow.contains("--no-verify"),
        "publish workflow should publish the same verified package path without skipping verification"
    );

    let validate_step = workflow
        .find("- name: Validate committed release version")
        .expect("publish workflow should validate committed release versions");
    let discover_step = workflow
        .find("- name: Discover unpublished versions")
        .expect("publish workflow should discover unpublished versions");

    assert!(
        validate_step < discover_step,
        "release version validation must happen before crates.io discovery"
    );

    for expected in [
        "$env:GITHUB_REF_TYPE -ne \"tag\"",
        "git diff --exit-code",
        "cargo metadata --locked --no-deps --format-version 1",
        "github.event.inputs.version",
        "cargo publish --dry-run --locked --package $name",
        "cargo publish --locked --package $name",
    ] {
        assert!(
            workflow.contains(expected),
            "publish workflow should contain `{expected}`"
        );
    }
}

#[test]
fn release_workflow_prepares_release_commit_tag_and_dispatches_publish() {
    let workflow = read_workspace_file(".github/workflows/release-crates.yml");

    for expected in [
        "workflow_dispatch:",
        "version:",
        "target_branch:",
        "contents: write",
        "actions: write",
        "RELEASE_VERSION",
        "cargo update --workspace",
        "cargo test --locked -- --test-threads=1",
        "git commit -m \"chore: release v$version\"",
        "git tag -a $tagName",
        "git push origin $tagName",
        "gh workflow run publish-crates.yml --ref $tagName --field \"version=$version\"",
    ] {
        assert!(
            workflow.contains(expected),
            "release workflow should contain `{expected}`"
        );
    }
}

fn read_workspace_file(path: &str) -> String {
    let file_path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), path);
    fs::read_to_string(&file_path).unwrap_or_else(|err| panic!("{file_path} should be readable: {err}"))
}
