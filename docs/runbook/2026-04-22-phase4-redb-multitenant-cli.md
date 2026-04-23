---
updated_at: 2026-04-22T10:05:00-0400
status: completed
---

# Phase 4A/4B/4C Redb Persistent Store + Multi-Tenant CLI Parity

## Scope
Implemented Phase 4 persistent store foundations using `redb` and extended CLI file/db parity for the Phase 3 command surface.

## Contract (implemented)
- Persistent quad store behavior matches Mem store semantics for insert/remove/query-pattern/stats.
- Data persists across reopen.
- A single redb file supports multiple isolated datasets via `--dataset`.
- Dataset scoping applies to dictionary and index lookups.

## Red tests added first

### Store integration
- `crates/rdf-store/tests/redb_store_integration.rs`
  - `redb_persists_quads_across_reopen`
  - `redb_insert_delete_and_idempotence`
  - `redb_isolates_datasets_in_one_database`

### Store property/model-based
- `crates/rdf-store/tests/redb_store_properties.rs`
  - `model_based_sequences_match_reference_model_for_single_dataset`

### CLI integration
- `crates/rdf-cli/tests/cli.rs`
  - `ingest_writes_to_db_source_and_query_pattern_reads_it`
  - `db_source_isolates_query_results_by_dataset`
  - `dump_and_stats_support_db_source`
  - `query_pattern_parity_between_file_and_db_source`

All of these failed initially (missing `RedbQuadStorage`, missing `--dataset`, no DB-source routing).

## Implementation to green

### `rdf-store`
- Added `RedbQuadStorage` in `crates/rdf-store/src/redb_store.rs`, exported from `crates/rdf-store/src/lib.rs`.
- Added persistent dataset catalog and per-dataset counters:
  - `meta`
  - `dataset_by_name`
  - `next_term_by_dataset`
- Added per-dataset term dictionary tables:
  - `term_to_id`
  - `id_to_term`
- Added six dataset-prefixed quad indexes:
  - `d_gspo`, `d_gpos`, `d_gosp`
  - `d_spog`, `d_posg`, `d_ospg`
- Implemented bounded range scans over selected index order and post-filtered pattern matching.
- Added deterministic regression test for discovered bug:
  - `redb_empty_dataset_remove_and_scan_are_stable`
  - This guards against table-not-created failures on empty datasets.

### `rdf-cli`
- Added `--dataset` (default `default`) for:
  - `ingest`
  - `query-pattern`
  - `dump`
  - `stats`
- Added DB/file routing:
  - If path extension is `.nt/.nq/.ttl/.trig` => file mode
  - Otherwise => redb DB mode
- `ingest` now writes to file formats or inserts into redb datasets.
- `query-pattern`, `dump`, `stats` now operate against either file source or DB source.

## Invariants now guarded
- Insert idempotence and delete correctness in persistent store.
- Persistence across process reopen.
- Strong dataset isolation within one DB file.
- File-vs-DB query-pattern parity for same dataset content.
- Empty-dataset operations are stable and deterministic.

## Validation
- `cargo test -p rdf-store --test redb_store_integration --test redb_store_properties`
- `cargo test -p rdf-cli --test cli`
- `cargo test -p rdf-store -p rdf-cli`
- `cargo clippy -p rdf-store -p rdf-cli --all-targets -- -D warnings`

All commands passed.
