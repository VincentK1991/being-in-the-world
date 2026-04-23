---
updated_at: 2026-04-22T02:15:00-0400
status: completed
---

# Phase 3A/3B In-Memory Store + CLI v0.1

## Summary
- Implemented `MemQuadStorage` in `rdf-store` with three BTree indexes over encoded IDs:
  - `SPOG`
  - `POSG`
  - `OSPG`
- Added public store API for Phase 3 behavior:
  - `QuadPattern`
  - `StoreStats`
  - `MemQuadStorage::{insert,remove,quads_matching,all_quads,stats}`
- Upgraded CLI to cover Phase 3 file-source workflows:
  - `query-pattern --source ... --subject/--predicate/--object/--graph`
  - `stats --source ...`
  - `dump --source ... --format ...` now serializes real RDF output
  - `ingest --from ... --to ...` now performs real format conversion and writes output file

## Red -> Green TDD Loop

### Red tests added first
- `crates/rdf-store/tests/mem_store_integration.rs`
  - insert/query consistency and idempotence
  - delete correctness
  - graph-scoped default vs named graph filtering
- `crates/rdf-store/tests/mem_store_properties.rs`
  - model-based operation sequence test (`Insert`/`Remove`/`Query`) against a `HashSet<Quad>` reference model
  - deterministic repeated query behavior
- `crates/rdf-cli/tests/cli.rs` additions
  - `query_pattern_matches_rdf_type_in_turtle_source`
  - `dump_outputs_nquads_for_turtle_source`
  - `stats_reports_quad_and_distinct_predicate_counts`
  - `ingest_converts_turtle_to_nquads_output_file`
  - `help_lists_core_subcommands` extended for new commands

### Minimal implementation to pass
- Added term-ID encoding + cache-backed lookups in `MemQuadStorage`.
- Added index-select strategy for query patterns using longest bound prefix across SPOG/POSG/OSPG and post-filter on full pattern.
- Added CLI source parsing/serialization helpers for `.nt`, `.nq`, `.ttl`, `.trig` paths.
- Added token parser for `query-pattern` selectors with support for:
  - wildcard variables (`?x`)
  - IRIs (`<...>`, `http://...`, `https://...`)
  - prefixed names for `rdf:`, `rdfs:`, `xsd:`
  - blank nodes (`_:`)
  - simple string literals (`"..."`)
  - graph token `default`

## Regression guards now in place
- Duplicate insertions do not create duplicate logical quads.
- Removing one quad does not affect unrelated quads.
- Pattern queries remain deterministic and model-consistent across generated operation sequences.
- CLI conversion writes parseable output RDF and reports expected counts.

## Validation
- `cargo test -p rdf-store --all-targets`
- `cargo test -p rdf-cli --test cli`
- `cargo test -p rdf-store -p rdf-cli -p rdf-testkit`
- `cargo clippy --workspace --all-targets -- -D warnings`

All commands passed.
