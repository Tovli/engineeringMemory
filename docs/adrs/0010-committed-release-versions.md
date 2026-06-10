# ADR-0010: Publish crates from committed semantic release versions

- **Status:** Accepted
- **Date:** 2026-06-10
- **Milestone:** Release Automation
- **Context refs:** `.github/workflows/publish-crates.yml`, `Cargo.toml`, `Cargo.lock`, `tests/publish_workflow.rs`, crates.io publish workflow

## Context
The publish workflow currently tries to make every publishable build unique by editing crate versions in
CI from `GITHUB_RUN_NUMBER`. That made the package version unpublished, but it also meant the crate being
verified and published no longer matched the checked-out git commit.

Cargo protects this path by default: `cargo publish` refuses to package tracked files that contain
uncommitted changes. The immediate workaround is `--allow-dirty`, but that weakens the safety guard that
prevents accidental publication of unexpected generated or modified files.

The project needs publish automation, but the release artifact should remain traceable to source control.
For crates.io consumers, the version in `Cargo.toml`, the package contents, and the git history should
agree without requiring CI-only mutation.

## Decision
Use committed semantic release versions as the source of truth.

- Human or release automation updates `Cargo.toml` and `Cargo.lock` to the intended semver version.
- The version bump is committed before publishing.
- Publishing is automated from a clean checkout of that commit.
- Release tags use the crate version, for example `v0.1.0`.
- The publish workflow validates that the tag version matches the committed `Cargo.toml` version before
  publishing.
- CI no longer derives crate versions from `GITHUB_RUN_NUMBER`.
- CI no longer uses `--allow-dirty` for publish, because publishing should happen from a clean tree.

The automation path is:

1. Update the package version in source control.
2. Commit the release version.
3. Create and push a matching `vX.Y.Z` tag.
4. GitHub Actions verifies tests and `cargo publish --dry-run --locked`.
5. GitHub Actions publishes any unpublished packages with `cargo publish --locked`.

Manual `workflow_dispatch` may remain available for maintainers, but it must perform the same clean-tree
and version-consistency checks.

## Consequences
- **+** crates.io package versions are directly traceable to git commits and tags.
- **+** Cargo's default dirty-tree protection remains active during verification and publish.
- **+** Failed publishes can be retried without changing the package version.
- **+** Release automation is still possible through a tag-driven workflow or a release tool that creates
  the version bump commit and tag.
- **-** Publishing every push to `main`/`master` is no longer automatic unless that push is a release
  commit plus matching tag.
- **-** Contributors must follow a release step instead of relying on CI to invent patch versions.
- **-** A failed release after tagging may require deleting/replacing an unpublished git tag, or creating
  a follow-up release version if the crate was already published.
- **Future:** A release helper can automate the local/versioning side by preparing the version bump commit,
  changelog entry, and tag, but the publish workflow should still treat committed semver as the authority.
