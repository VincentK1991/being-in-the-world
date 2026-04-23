# RDF Store Feature Checklist

Use this checklist to track required RDF store features.
Rule: only check a box after the required tests for that feature pass.

## How To Use

1. Add a new unchecked item before implementation starts.
2. Link the tests that prove the feature (property/integration/conformance, plus unit tests when appropriate).
3. Keep the item unchecked while tests are failing.
4. Check the item only when all required tests are passing.

## Feature Checklist

- [x] Feature: Phase 1 RDF model language-tag and dictionary round-trip semantics
  - Status: Completed
  - Required tests:
    - Property: `crates/rdf-model/src/dictionary.rs::tests::encode_decode_round_trip`
    - Property: `crates/rdf-model/tests/properties_language_tags.rs::rejects_numeric_primary_subtags`
    - Conformance: `crates/rdf-model/tests/conformance_language_tags.rs`
  - Notes: Guards RDF 1.1 language-tag validation and encode/decode invariants for Phase 1 core model terms.

- [x] Feature: Manifest smoke runner emits concrete pass/fail status
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-cli/tests/cli.rs::test_suite_reports_manifest_smoke_status_counts`
    - Conformance: `crates/rdf-testkit/src/manifest.rs::tests::smoke_runner_reports_passed_and_failed_cases`
  - Notes: Replaces all-pending test-suite reporting with non-empty file smoke assertions for discovered manifest entries.

- [x] Feature: Phase 2A N-Triples parser/serializer with conformance and property gates
  - Status: Completed
  - Required tests:
    - Conformance: `crates/rdf-io/tests/ntriples_conformance.rs`
    - Property: `crates/rdf-io/tests/ntriples_properties.rs`
    - Conformance: `crates/rdf-testkit/tests/ntriples_manifest.rs`
    - Integration: `crates/rdf-cli/tests/cli.rs::test_suite_reports_ntriples_conformance_counts`
    - Integration: `crates/rdf-cli/tests/cli.rs::{ingest_parses_ntriples_input_and_reports_count,ingest_fails_on_invalid_ntriples_input}`
  - Notes: Adds strict line/column parse errors, `.pos.nt`/`.neg.nt` conformance routing, and CLI ingest/test-suite wiring for N-Triples.

- [x] Feature: Phase 2B N-Quads parser/serializer with conformance and property gates
  - Status: Completed
  - Required tests:
    - Conformance: `crates/rdf-io/tests/nquads_conformance.rs`
    - Property: `crates/rdf-io/tests/nquads_properties.rs`
    - Conformance: `crates/rdf-testkit/tests/manifest_formats.rs::nquads_runner_enforces_positive_and_negative_expectations`
    - Integration: `crates/rdf-cli/tests/cli.rs::{test_suite_reports_nquads_conformance_counts,ingest_parses_nquads_input_and_reports_count,ingest_fails_on_invalid_nquads_input}`
  - Notes: Extends strict line/column parsing, serialization, and `.pos.nq`/`.neg.nq` conformance routing.

- [x] Feature: Phase 2C Turtle parser/serializer with graph-isomorphism property gates
  - Status: Completed
  - Required tests:
    - Conformance: `crates/rdf-io/tests/turtle_conformance.rs`
    - Property: `crates/rdf-io/tests/turtle_properties.rs`
    - Conformance: `crates/rdf-testkit/tests/manifest_formats.rs::turtle_runner_enforces_positive_and_negative_expectations`
    - Integration: `crates/rdf-cli/tests/cli.rs::{test_suite_reports_turtle_conformance_counts,ingest_parses_turtle_input_and_reports_count,ingest_fails_on_invalid_turtle_input}`
  - Notes: Uses `logos` tokenization with `winnow`-validated prefixed-name parsing and semantic (isomorphic) round-trip testing.

- [x] Feature: Phase 2D TriG parser/serializer with graph-isomorphism property gates
  - Status: Completed
  - Required tests:
    - Conformance: `crates/rdf-io/tests/trig_conformance.rs`
    - Property: `crates/rdf-io/tests/trig_properties.rs`
    - Conformance: `crates/rdf-testkit/tests/manifest_formats.rs::trig_runner_enforces_positive_and_negative_expectations`
    - Integration: `crates/rdf-cli/tests/cli.rs::{test_suite_reports_trig_conformance_counts,ingest_parses_trig_input_and_reports_count,ingest_fails_on_invalid_trig_input}`
  - Notes: Adds graph-block parsing/serialization and `.pos.trig`/`.neg.trig` conformance routing.

- [x] Feature: Phase 3A MemQuadStorage BTree index semantics with property/model-based correctness
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-store/tests/mem_store_integration.rs`
    - Property: `crates/rdf-store/tests/mem_store_properties.rs`
  - Notes: Adds in-memory SPOG/POSG/OSPG indexing, idempotent insert/delete semantics, graph-scoped pattern matching, and deterministic query output for BGP-of-one patterns.

- [x] Feature: Phase 3B CLI `query-pattern`/`dump`/`stats` and ingest conversion for file sources
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-cli/tests/cli.rs::{query_pattern_matches_rdf_type_in_turtle_source,dump_outputs_nquads_for_turtle_source,stats_reports_quad_and_distinct_predicate_counts,ingest_converts_turtle_to_nquads_output_file,ingest_converts_turtle_to_ntriples_output_file}`
    - Integration: `crates/rdf-cli/tests/cli.rs::help_lists_core_subcommands`
  - Notes: Adds Phase 3 CLI boundaries for basic quad-pattern querying and source stats over file-backed RDF inputs, and makes `ingest` perform real serialization conversion.

- [x] Feature: Phase 3C verification gates (CLI snapshots + load benchmark)
  - Status: Completed
  - Required tests:
    - Snapshot: `crates/rdf-cli/tests/cli_snapshots.rs::{snapshot_query_pattern_rdf_type,snapshot_dump_nquads,snapshot_stats_basic_ttl}`
    - Benchmark: `crates/rdf-store/benches/mem_store_load.rs` via `cargo bench -p rdf-store --bench mem_store_load -- --quick`
  - Notes: Locks CLI v0.1 output formatting with `insta` and verifies the 1M-quad in-memory load benchmark path executes successfully.

- [x] Feature: Phase 4A RedbQuadStorage persistent dictionary/index semantics
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-store/tests/redb_store_integration.rs::{redb_persists_quads_across_reopen,redb_insert_delete_and_idempotence,redb_empty_dataset_remove_and_scan_are_stable}`
    - Property: `crates/rdf-store/tests/redb_store_properties.rs::model_based_sequences_match_reference_model_for_single_dataset`
  - Notes: Requires committed quad persistence across process restarts and parity of insert/delete/query semantics with in-memory behavior.

- [x] Feature: Phase 4B Multi-tenant dataset isolation within a single redb file
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-store/tests/redb_store_integration.rs::redb_isolates_datasets_in_one_database`
    - Integration: `crates/rdf-cli/tests/cli.rs::db_source_isolates_query_results_by_dataset`
  - Notes: Enforces strict per-dataset dictionary and index scoping.

- [x] Feature: Phase 4C CLI DB-source parity for `ingest`/`query-pattern`/`dump`/`stats`
  - Status: Completed
  - Required tests:
    - Integration: `crates/rdf-cli/tests/cli.rs::{ingest_writes_to_db_source_and_query_pattern_reads_it,dump_and_stats_support_db_source}`
    - Integration: `crates/rdf-cli/tests/cli.rs::query_pattern_parity_between_file_and_db_source`
  - Notes: Uses `--source ./mydb --dataset <name>` and keeps file/db behavior aligned for Phase 3 command surface.

## Optional Completion Metadata

Use this section after checking a feature:

- Completed by:
- Date: 2026-04-22
- Test command(s): `cargo test -p rdf-model`; `cargo check -p rdf-model --all-targets --features rdf-star`; `cargo test -p rdf-io --all-targets`; `cargo test -p rdf-testkit -p rdf-cli`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo nextest run --workspace --all-targets`; `cargo test --workspace --all-targets`; `cargo deny check`
- PR/commit reference:
