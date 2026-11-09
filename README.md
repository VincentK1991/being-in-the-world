# Local RDF store

A knowledge graph for AI applications that you can query in one language. Semantic search, graph
queries and logical inference sit behind a single SPARQL interface, so you can ground an LLM or an
agent in facts that are searchable by meaning, derived by rules and traceable to their source. It is
an RDF store built on [Oxigraph](https://github.com/oxigraph/oxigraph) (RocksDB), with a small,
standard core and a plugin architecture for the rest.

## What it aims to do

Retrieval pipelines usually end up stitching together a vector database, a graph database, a reasoner
and a validator, each with its own query language and its own idea of what is current. This project
aims to make that one system:

- **Vector, graph and rules in one query.** Semantic and hybrid search are SPARQL property functions,
  and inference results, SHACL reports and virtual SQL tables are ordinary graphs. One query can find
  entities by meaning, follow relationships, and use facts that were inferred rather than stored.
- **Answers you can prove.** The reasoner can show why a triple holds, as a proof traced back to the
  asserted data, and inconsistencies come back as results rather than silent failures. Context handed to
  a model can carry its provenance.
- **Asserted and derived data stay apart.** Inferred triples, reports and indexes live in separate
  derived graphs together with a record of the inputs they were built from, so you can tell when they
  are stale and never confuse them with what you stated.
- **Standards, no lock-in.** Plain RDF, SPARQL, OWL and SHACL on open formats, with W3C conformance
  baselines that can only move up. Your data and queries move to any other triple store.
- **A small core.** The engine does storage, parsing, the query policy and derived-graph bookkeeping.
  Anything that needs the network or heavy dependencies is a plugin or a sidecar.

**For AI agents:** `rdf-mcp` is a read-only MCP server. Its tools let an agent discover datasets,
schemas, property functions and `SERVICE` endpoints, then query with SPARQL, with a dry run that
flags misspelt terms. Results say which facts are asserted and which are inferred, and the reasoner
can explain an inferred fact. One query can combine semantic search, inferred facts, the SQL virtual
graph, other datasets and SHACL reports. Property tests check that the server's rewrites keep the
meaning of the agent's query. See [`crates/rdf-mcp`](crates/rdf-mcp/README.md).

## Components

| Component | What it is |
| --- | --- |
| `rdf-store` | Oxigraph and RocksDB storage, datasets, backup and restore |
| `rdf-engine` | The database library: query policy, property-function rewrite, derived graphs and refresh rules |
| `rdf-server` | Axum HTTP server for one dataset, and a gateway that serves many at `/d/<name>/` |
| `rdf-cli` | The `rdftool` command line |
| `rdf-mcp` | Read-only MCP server for AI agents: orient tools plus SPARQL over every plugin |
| `rdf-plugin-api` | The plugin SDK (v3), with no HTTP, Oxigraph or filesystem dependency |
| Studio | Browser UI (Astro, React) for querying, browsing, plugins, and the documentation at `/docs/` |
| Plugins | `semantic-search` (semantic and hybrid), `inference` (OWL 2 RL, explanations), `shacl`, `federation` and virtual graph, `catalog` |
| Sidecars | Separate containers the plugins call: Qdrant and embeddings, OWL-RL and Jena reasoners, Jena SHACL, Ontop over Postgres, Comunica |

## Status

This is work in progress and **not yet a completed release**. The storage engine, query surface,
plugins and Studio work end to end on a local stack and are covered by about 200 Rust tests (property tests included), browser
tests, W3C conformance baselines and a nightly live-stack run, but the project has no authentication or quotas (the server only binds
to loopback), conformance is partial (for example SPARQL 1.1 at about 82% of the W3C suite), and
interfaces may still change. See [validation status](docs/runbook/2026-11-01-oxigraph-validation.md)
for the open gates, [LICENSE](LICENSE) (Apache-2.0) with [third-party notices](THIRD_PARTY_NOTICES.md), and the documentation (`/docs/` in Studio, sources in
`apps/studio/src/content/docs/`) for what it is, how to use it, how it works inside, and background on ontologies, OWL profiles, SHACL, reasoners and federation.

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

## Connect an AI agent (MCP)

With the stack running, build the read-only MCP server and register it with your agent:

```sh
cargo build --release -p rdf-mcp
claude mcp add rdf -- "$PWD/target/release/rdf-mcp"   # or point any MCP client at the binary (stdio)
```

It talks to the gateway on `127.0.0.1:7878` (`RDF_MCP_GATEWAY`; only loopback addresses are accepted).
`just mcp-test` runs its live suites, including 11 queries that each combine several plugins. See
[`crates/rdf-mcp`](crates/rdf-mcp/README.md) and the docs page "Connect an AI agent (MCP)".

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
