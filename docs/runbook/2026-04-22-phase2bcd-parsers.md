---
updated_at: 2026-04-22T00:42:00-0400
status: completed
---

# Phase 2B/2C/2D Parsers and Conformance

## Summary
- Completed remaining Phase 2 parser/serializer milestones after Phase 2A:
  - Phase 2B: N-Quads
  - Phase 2C: Turtle
  - Phase 2D: TriG
- Added new public `rdf-io` APIs:
  - N-Quads:
    - `parse_nquads_reader`, `parse_nquads`
    - `serialize_nquads`, `serialize_nquads_to_string`
  - Turtle:
    - `parse_turtle_reader`, `parse_turtle`
    - `serialize_turtle`, `serialize_turtle_to_string`
  - TriG:
    - `parse_trig_reader`, `parse_trig`
    - `serialize_trig`, `serialize_trig_to_string`
- Added strict error surfaces for Turtle/TriG:
  - `TurtleErrorKind`, `TurtleError` (aliased for TriG)
  - line/column diagnostics and deterministic error categories.
- Implemented Turtle/TriG tokenization using `logos` and integrated `winnow`-based validation for prefixed-name and prefix namespace segments.
- Extended `rdf-testkit` conformance routing:
  - `run_manifest_nquads`
  - `run_manifest_turtle`
  - `run_manifest_trig`
  - `run_manifest` now dispatches format-specific conformance for `.pos.*`/`.neg.*` fixtures and retains smoke fallback for unmarked manifests.
- Wired `rdftool ingest` for `.nq`, `.ttl`, `.trig`:
  - parse validation + parsed quad counts on success
  - non-zero exit with detailed parse diagnostics on failure.

## Red/Green Test Additions
- `rdf-io` conformance tests:
  - `tests/nquads_conformance.rs`
  - `tests/turtle_conformance.rs`
  - `tests/trig_conformance.rs`
- `rdf-io` property tests:
  - `tests/nquads_properties.rs`
  - `tests/turtle_properties.rs`
  - `tests/trig_properties.rs`
- Added graph-isomorphism helper for semantic round-trip checks:
  - `tests/common.rs`
- `rdf-testkit` format runner tests:
  - `tests/manifest_formats.rs`
- Extended `rdf-cli` integration tests:
  - conformance summaries for N-Quads/Turtle/TriG
  - ingest success/failure behavior for each format.

## Fixture Coverage
- Added W3C-style positive/negative fixture trees:
  - `crates/rdf-testkit/tests/fixtures/w3c-nquads`
  - `crates/rdf-testkit/tests/fixtures/w3c-turtle`
  - `crates/rdf-testkit/tests/fixtures/w3c-trig`
- Added corresponding CLI fixture subsets for integration tests:
  - `crates/rdf-cli/tests/fixtures/w3c-nquads`
  - `crates/rdf-cli/tests/fixtures/w3c-turtle`
  - `crates/rdf-cli/tests/fixtures/w3c-trig`

## Validation
- `cargo fmt --all`
- `cargo test -p rdf-io --all-targets`
- `cargo test -p rdf-testkit -p rdf-cli`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo deny check`

All commands passed.
