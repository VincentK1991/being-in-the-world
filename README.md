# Local RDF store

Rust CLI and localhost service backed by pinned Oxigraph/RocksDB. Supports
N-Triples, N-Quads, Turtle, TriG, SPARQL queries and updates, isolated logical
datasets, legacy redb migration, and per-dataset backup/restore.

The core implementation is under validation, **not yet a completed release**.
See [validation status](docs/runbook/2026-11-01-oxigraph-validation.md) for open
dependency and conformance gates. Reasoning, vectors and virtual SQL are deferred.

## Linux / WSL setup

Rust 1.93 or later is required. Install native dependencies and validation tools:

```sh
bash scripts/setup-linux.sh
cargo build --locked -p rdf-cli
```

RocksDB's initial C++ build can take several minutes. Normal builds/tests can
use `--offline` after fetching dependencies. Native prerequisites are Clang,
libclang, CMake, pkg-config and a C++ compiler. There is no Docker requirement.

## Local workflow

Run these commands from the repository root:

```sh
target/debug/rdftool ingest --from crates/rdf-cli/tests/fixtures/phase3/basic.ttl --to ./mydb
target/debug/rdftool query --source ./mydb 'SELECT ?s ?p ?o WHERE { ?s ?p ?o }'
target/debug/rdftool update --source ./mydb 'INSERT DATA { <urn:alice> <urn:likes> <urn:rdf> }'
target/debug/rdftool query-pattern --source ./mydb --predicate rdf:type
target/debug/rdftool dump --source ./mydb --format n-quads
target/debug/rdftool stats --source ./mydb
target/debug/rdftool backup --source ./mydb --to ./backup
target/debug/rdftool restore --from ./backup --to ./restored
```

Use `--dataset NAME` on database commands; the default is `default`. A root
catalog maps names to generated UUID directories, so datasets do not share graphs.
Read commands reject missing datasets. `ingest` and `serve` create them.
`rdftool datasets --db ROOT list|create|delete` manages them, and `rdftool gateway --db ROOT`
serves many datasets at `/d/<name>/` (see [ADR 0008](docs/adr/0008-dataset-isolation-gateway.md)).
RDF-file sources are loaded into a transient memory store. Updates require a DB
or HTTP source. `query` accepts inline text or `--query-file`; `update` also
accepts `--update-file`.

SELECT/ASK default to SPARQL Results JSON (`--format json|xml|csv|tsv`). Graph
results default to Turtle. Dumps accept `n-quads|nquads`, `n-triples|ntriples`,
`turtle`, and `trig`. Triples-only exports reject named graphs. Machine results
go to stdout; ingest status goes to stderr. This intentionally changes the old
CLI output format. Stats output is JSON.

Ordinary ingestion rolls back malformed input. `--bulk --memory-mb 256 --threads 2`
opts into the engine bulk loader, which may leave partial progress on failure.
The memory setting is a loader target, not a hard process memory limit.

## RDF Studio browser UI

Studio uses **Astro + React + TypeScript + Tailwind**. Axum is a separate HTTP
backend, calling the public `rdf-engine` database library. Plugins depend on a
separate SDK. Read [the architecture contract](docs/adr/0002-engine-plugins-and-studio.md)
and [plugin development guide](plugins/README.md).

The default application also registers **custom SPARQL properties**. Choose
**Custom property · tokens** in Studio to generate token/position rows, or use
`(?token ?position) <urn:rdftool:property:tokens> "hello world"` in a query pattern.
See [defining new properties](plugins/custom-properties/README.md) for the SDK,
examples, limits, and the distinction from future vector/BM25 search plugins.

**Semantic search is available as an opt-in plugin using Qdrant and a local CPU/GPU
embedding sidecar.** See the [semantic-search setup and SPARQL examples](plugins/semantic-search/README.md).
The plugin supports matching literal text and returning distinct subjects or
objects. BM25 remains a separate future extension.

```sh
# Node >=22.23.4 (apps/studio/.nvmrc), then:
npm ci --prefix apps/studio
npm run check --prefix apps/studio
npm run build --prefix apps/studio
cargo build --locked -p rdf-cli
target/debug/rdftool serve --db ./mydb --dataset default --bind 127.0.0.1:7878
```

Open **http://127.0.0.1:7878/**. The built frontend assets are embedded in the
binary; no Node server is needed at runtime. Rebuild the frontend, then Rust,
after UI changes. A Rust-only build without frontend assets still serves the
database API and explains the missing UI build at `/`.

For frontend development, keep Axum running and use `npm run dev --prefix
apps/studio`. Astro serves http://127.0.0.1:4321 and proxies database requests to
port 7878 (`RDF_BACKEND_URL` overrides that target).

- Import `.ttl`, `.nt`, `.trig`, or `.nq` files by choosing or dropping a file.
  Turtle/N-Triples can target an optional named graph. Imports append and commit
  atomically; a malformed file leaves the dataset unchanged.
- Run SELECT, ASK, CONSTRUCT, or DESCRIBE in the SPARQL editor. Results have table
  and raw views, pagination, downloads, and query history saved in this browser.
- Inspect quad counts and named graphs, or load the included example and run the
  people/project join. Uploaded data survives server restarts.

The workbench uses the server's selected dataset. Stop and restart with another
`--dataset` to switch. Uploads default to a 16 MiB limit; for larger files use the
CLI ingestion workflow. The editor is for queries; SPARQL updates remain available
through the CLI and `/update`. Results are buffered in memory; pagination is for
display, so use `LIMIT` for large queries.

See [browser behavior tests](docs/runbook/rdf-studio.md) for repeatable validation
and the relationship to the independent Jena checks.

## HTTP

```sh
target/debug/rdftool serve --db ./mydb --bind 127.0.0.1:7878
target/debug/rdftool query --source http://127.0.0.1:7878 'ASK { ?s ?p ?o }'
```

The service exposes `/query`, `/update`, `/store?graph=IRI` or `/store?default`,
`/health`, and `/metrics`, plus the workbench at `/`, `GET /api/status`, and
`POST /api/import` (raw RDF body; optional `?graph=IRI`). It binds only to loopback,
with no CORS enabled.
Defaults: 16 MiB request bodies and four concurrent engine operations;
configure using `--max-body-bytes` and `--concurrency`. Excess work receives 503.
One server exposes one dataset. HTTP clients use that server's dataset, not a
CLI `--dataset` override. Stop the server before opening the same dataset from
another process. SIGINT/SIGTERM drain accepted requests; long queries are not
claimed to be forcibly cancelled.

SERVICE (including SILENT) and LOAD are rejected. Use explicit ingestion instead.
The HTTP layer currently rejects protocol dataset-selection parameters; use
SPARQL FROM/GRAPH clauses. Full protocol conformance is an open release gate.

## Migration

```sh
target/debug/rdftool migrate --from-redb ./legacy.redb --to ./migrated
```

Migration locks the source, opens a disposable snapshot with redb, and exports
every dataset, including empty ones. It verifies graph isomorphism before
publishing a staged destination. The source remains unchanged. Migration and
restore refuse existing destinations. Backups are native single-dataset engine
directories; restore publishes a new catalogued root.

## Validation

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
cargo deny check
bash scripts/conformance.sh
cargo run --release -p rdf-store --example persistent_benchmark -- 100000
cargo run --release -p rdf-store --example persistent_benchmark -- 1000000
```

Live HTTP tests need permission to bind localhost. `test-suite --manifest FILE`
interprets RDF manifests and reports passed/failed/unsupported/skipped cases.
`smoke-suite` is explicitly only file-presence checking. A report with unsupported
cases is incomplete, even if no cases failed. Official suites are pinned in
the conformance script. `diff --query QUERY_FILE --sources SOURCE1,SOURCE2`
compares result multiplicity, explicit ordering, boolean values and graph
isomorphism; execution errors are reported separately from mismatches.

## Library

`rdf_store::engine::Dataset` exposes memory/persistent construction, ingestion,
queries, updates, dump and backup; `rdf_store::oxigraph` re-exports engine types.
The legacy `rdf-model`/`rdf-io`/redb implementations remain for migration and
regression coverage. Custom SPARQL placeholder crates are outside the workspace.
See [the architecture decision](docs/adr/0001-oxigraph-core.md).

Embedding providers can now be selected per semantic index, including local CPU/GPU
and OpenAI-compatible endpoints. See [provider configuration](plugins/semantic-search/README.md#multiple-embedding-providers),
[executed SPARQL examples](plugins/semantic-search/examples/README.md), and
[provider validation](docs/runbook/2026-11-02-embedding-providers.md).

Graph-scoped RDFS/OWL-rule inference is available through the [inference plugin](plugins/inference/README.md), with [executed SPARQL examples](plugins/inference/examples/README.md).

The inference plugin also supports [SPARQL CONSTRUCT and Jena custom-rule providers](plugins/inference-providers/README.md), with [executed examples](plugins/inference/examples/rules/README.md).
