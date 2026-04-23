---
updated_at: 2026-04-21T23:55:00-0400
status: completed
---

# Phase 2A N-Triples Parser + Conformance

## Summary
- Implemented strict N-Triples parsing and serialization in `crates/rdf-io`:
  - public parser APIs:
    - `parse_ntriples_reader`
    - `parse_ntriples`
  - public serializer APIs:
    - `serialize_ntriples`
    - `serialize_ntriples_to_string`
  - detailed parse errors with:
    - `NTriplesErrorKind`
    - `line`
    - `column`
    - message text
- Added red/green tests before implementation:
  - conformance tests for valid/invalid syntax and error positioning
  - property tests for parse/serialize semantic round-trip and determinism
- Upgraded `rdf-testkit` conformance execution:
  - added `run_manifest_ntriples`
  - added `run_manifest` dispatcher (N-Triples-aware, smoke fallback)
  - naming convention:
    - `*.pos.nt` => parser must succeed
    - `*.neg.nt` => parser must fail
  - added optional diagnostics in `ManifestResult.details`
- Wired `rdftool`:
  - `test-suite` now uses format-aware `run_manifest`
  - `ingest` validates `.nt` input using parser and reports parsed triple count
  - parse failures return non-zero with line/column/kind diagnostics
- Added curated in-repo fixture sets for N-Triples positive/negative cases in:
  - `crates/rdf-testkit/tests/fixtures/ntriples`
  - `crates/rdf-cli/tests/fixtures/w3c-ntriples`

## Validation
- `cargo fmt --all`
- `cargo test -p rdf-io -p rdf-testkit -p rdf-cli`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo deny check`

All commands passed.
