---
updated_at: 2026-04-21T23:18:00-0400
status: completed
---

# Manifest Smoke Status Runner

## Summary
- Added concrete manifest execution statuses in `crates/rdf-testkit`:
  - `ManifestStatus::Passed`
  - `ManifestStatus::Failed`
  - retained `ManifestStatus::Pending` for compatibility.
- Implemented `run_manifest_smoke(path)` in `crates/rdf-testkit/src/manifest.rs`:
  - enumerates supported manifest entries
  - asserts each entry is non-empty (whitespace-only files fail)
  - returns per-case pass/fail status.
- Updated `rdftool test-suite` to run the smoke runner and report:
  - `discovered`
  - `passed`
  - `failed`
  - `pending`.
- Updated CLI output rows to render status labels per case (`passed`, `failed`, `pending`).
- Added regression tests:
  - `rdf-testkit` unit test for mixed pass/fail smoke results
  - `rdf-cli` integration test for `test-suite` summary counts.

## Validation
- `cargo fmt --all`
- `cargo test -p rdf-testkit -p rdf-cli`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace --all-targets`
- `cargo test --workspace --all-targets`

All commands passed.
