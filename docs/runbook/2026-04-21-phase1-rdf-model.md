---
updated_at: 2026-04-21T23:02:00-0400
status: completed
---

# Phase 1 RDF Model

## Summary
- Implemented Phase 1 in `crates/rdf-model` with concrete RDF domain types:
  - `NamedNode`, `BlankNode`, `Literal`
  - `Subject`, `Predicate`, `Object`, `GraphName`
  - `Triple`, `Quad`, `Term`
  - `QuotedTriple` behind `rdf-star` feature flag
- Added IRI validation using `oxiri`.
- Added literal/datatype vocabulary constants:
  - `xsd:string`, `xsd:boolean`, `xsd:integer`, `rdf:langString`.
- Implemented `TermId` with inline encoding scheme:
  - dictionary-backed IDs
  - inline booleans
  - inline signed i60 integers
  - inline short strings (<= 7 bytes)
- Implemented dictionary abstraction:
  - `Dictionary` trait
  - `InMemoryDictionary` HashMap-backed implementation
  - inline encode/decode fast path for canonical simple literals.
- Added unit tests and property tests:
  - model construction/validation tests
  - IRI parser edge-case tests
  - term-id inline round-trips and bounds
  - dictionary encode/decode round-trip (`proptest`)
- Added explicit Phase 1 conformance/property guardrails for language tags:
  - conformance test for valid BCP47-like tags and rejection of numeric primary subtags
  - property test ensuring numeric primary subtags are always rejected
- Hardened `Literal::new_language_tagged` validation:
  - primary language subtag must be ASCII alphabetic (still normalizing to lowercase)
- Verified feature build for `rdf-star`.

## Validation
- `cargo fmt --all`
- `cargo check -p rdf-model --all-targets`
- `cargo test -p rdf-model`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace --all-targets`
- `cargo check -p rdf-model --all-targets --features rdf-star`
- `cargo test --workspace --all-targets`
- `cargo nextest run --workspace --all-targets`
- `cargo deny check`

All commands passed.
