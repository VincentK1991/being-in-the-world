# Local validation — 2026-10-31

Checkout baseline: `1e9fe64` (phase 4). Environment: Linux/WSL,
rustc/cargo 1.93.1. Installed cargo-nextest 0.9.150 and cargo-deny 0.20.2.
Fetched locked dependencies; builds and tests can now run offline.

## Reproduce

Run from the repository root:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo nextest run --workspace --all-targets --locked --offline
cargo deny check
cargo check -p rdf-model --features rdf-star --offline
cargo bench -p rdf-store --bench mem_store_load --offline -- --quick
```

The dependency advisory check may need network access to refresh its database.
The equivalent `cargo test --workspace --all-targets --locked --offline`
also passed on the original lockfile.

## Dependency findings

The initial dependency check failed for RUSTSEC-2026-0190 (anyhow 1.0.102)
and RUSTSEC-2026-0204 (crossbeam-epoch 0.9.18). Updated only those packages
in Cargo.lock to 1.0.103 and 0.9.20 respectively. The subsequent advisory,
bans, licenses, and sources checks all passed.

After the lockfile updates, formatting, workspace/all-target compilation,
Clippy with warnings denied, and nextest all passed again: 100 tests passed,
0 skipped, in 28.605 seconds. `git diff --check` also passed.

## Manual CLI validation

Using `crates/rdf-cli/tests/fixtures/phase3/basic.ttl`, separate processes:

- Ingested 2 quads into a temporary redb database, dataset `demo`.
- Reopened the database: stats reported 2 quads and 2 predicates.
- Queried `rdf:type`: returned Alice as a Person (1 match).
- Read a second dataset: 0 quads, confirming isolation for this example.
- Dumped both original quads with `--format n-quads`.
- Reingested the input: 0 new quads, confirming idempotence.

Example (choose a new database filename):

```sh
cargo run -p rdf-cli --offline -- ingest --from crates/rdf-cli/tests/fixtures/phase3/basic.ttl --to /tmp/rdf-demo.redb --dataset demo
cargo run -p rdf-cli --offline -- query-pattern --source /tmp/rdf-demo.redb --dataset demo --predicate rdf:type
cargo run -p rdf-cli --offline -- dump --source /tmp/rdf-demo.redb --dataset demo --format n-quads
```

## Scope and limitations

- Original nextest run: 100 passed, 0 skipped (includes placeholder marker tests
  and the benchmark smoke case).
- Release one-million-quad benchmark: approximately 5.69 seconds on the original
  lockfile. This quick local measurement includes the workload's construction
  and cleanup; it is not a persistent-ingestion throughput guarantee.
- `query`, `diff`, and `serve` were executed and only printed placeholder text;
  no SPARQL evaluation, result comparison, or HTTP listener exists yet.
- `--format nquads` is rejected; the accepted spelling is `n-quads`.
- Existing parser fixtures do not establish full W3C conformance. Turtle/TriG
  coverage, streaming ingestion, reasoning, vectors, and production scalability
  remain outside what this validation establishes.
