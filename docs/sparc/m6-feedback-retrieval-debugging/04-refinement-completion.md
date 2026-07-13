# M6 Feedback and Retrieval Debugging - Refinement and Completion

## Implemented
- Added the Feedback bounded context under `src/feedback`.
- Added `tovli feedback` and `tovli feedback-report`.
- Persisted retrieval evidence from both `search` and `ask`.
- Added export support through `feedback-report --export <path>`.
- Added service-level and CLI integration tests.
- Addressed review findings by adding report observations with run/rank/score detail, explicit no-good
  query CLI output, collision-resistant query/run ids, and all-or-nothing validation for multi-item
  feedback commands.
- Addressed follow-up review findings by printing observation details in `feedback-report`, logging empty
  search/ask runs, and moving multi-item feedback persistence into a repository-level batch transaction.

## Verification
- `cargo test --test feedback_workflow --no-default-features`
- `cargo test --test feedback_cli --no-default-features`
- `cargo test --no-default-features`
- `cargo check`
- Ruflo security scan: `security scan --check input-validation --path src/feedback`

## Notes
- `cargo fmt --check` still reports formatting drift in pre-existing files outside this feature. The
  touched feedback files and tests were checked directly with `rustfmt --check`.
- Feedback remains analysis-only. Any feedback-influenced ranking experiment requires a future ADR and
  an eval comparison gate.
