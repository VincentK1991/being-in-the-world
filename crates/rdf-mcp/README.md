# rdf-mcp

A read-only [MCP](https://modelcontextprotocol.io) server that lets an AI agent discover a dataset and
query it with SPARQL. It speaks MCP over stdio and talks to the gateway over loopback HTTP. It never
opens database files and never calls an endpoint that changes data.

```sh
cargo build --release -p rdf-mcp
claude mcp add rdf -- "$PWD/target/release/rdf-mcp"          # Claude Code
```

Configuration: `RDF_MCP_GATEWAY` (default `http://127.0.0.1:7878`; only loopback addresses are
accepted) and `RDF_MCP_TIMEOUT_SECONDS` (default 60). Logs go to stderr, filtered by `RUST_LOG`.

## Tools

| Tool | What it answers |
|---|---|
| `list_datasets` | Which datasets exist, and their size |
| `describe_dataset` | Graphs (asserted, inferred by which job, validation reports), inference jobs and freshness, search indexes, SHACL jobs, virtual graph, federation |
| `get_schema` | Classes and properties with usage counts (asserted vs inferred), domain, range, labels and a PREFIX block; `focus` + `samples` for one term in depth |
| `list_endpoints` | `SERVICE` targets the dataset may call, what each one is, and their main classes |
| `list_plugins` | Property functions with a ready-to-use example; plugin operations and which tool reaches them |
| `sparql_query` | Runs SELECT, ASK, CONSTRUCT or DESCRIBE, or checks the query with `dry_run`; `graphs` chooses asserted, inferred, all or default |
| `search_entities` | Natural-language text to IRIs, through the search indexes |
| `describe_entity` | Every triple about one IRI, each tagged asserted or inferred |
| `reasoning` | `status`, `explain` (proof of an inferred triple), `consistency`, `lineage`, `validation` (SHACL results) |

## How `sparql_query` works

1. The query is parsed with `spargebra`, the parser the engine uses, so syntax errors match. Updates
   are refused.
2. **Routing.** A query with `SERVICE` goes to the federation plugin, and only to endpoints on its
   allowlist. Any other query goes to the dataset's own `/query`.
3. **Graphs.** The engine's default graph is only the unnamed graph. Unless the query has its own
   `FROM`, the server adds `FROM` and `FROM NAMED` for the selected graphs. The default is every
   asserted and inferred named graph, so plain patterns see stored and inferred facts merged.
4. **Property functions.** The engine refuses property functions in a query with a dataset clause
   (ADR 0003 left graph context for them to a later design). The server does what the engine would
   do: it runs each call site once on its own, puts the rows in a `VALUES` table, and only then adds
   the dataset clause.
5. **Checks** on every call, and the whole answer of a dry run:
   - IRIs that occur nowhere in the dataset, with suggestions;
   - property functions that are unknown, have the wrong arity, or are given variable inputs;
   - graphs that do not exist;
   - `SERVICE` endpoints that are not allowed;
   - inferred graphs whose job is not `ready`.

6. **Printing.** The rewritten query is written out with `sparql::to_sparql`, not spargebra's own
   printer. spargebra 0.4.8 drops the braces around a `FILTER` in some nested positions (inside an
   `OPTIONAL` group, before `OPTIONAL` / `MINUS` / `BIND`), and re-parsing then widens the filter's
   scope. `to_sparql` puts each such filter back in its own group first. The property tests found
   this; `tests/sparql.rs` keeps the two shapes as regression tests.

Results come back as `columns` and `rows`. The terms are written in SPARQL syntax (`<iri>`, `"text"@en`,
`42`), so they can be pasted into the next query.

## Safety

- Read-only. Only the plugin operations in `client::READ_OPERATIONS` can be called. Rebuild,
  validate, test and every update are out of reach.
- Loopback only. The store has no authentication, so a non-loopback gateway URL is refused.
- Every IRI and name that goes into a URL or a query is validated. User text goes into queries only as
  an escaped string literal.
- Responses are capped at 4 MiB, rows at 1,000, and query text at 64 KiB.
- The server instructions tell the agent that returned values are data, not instructions.

## Tests

- `cargo test -p rdf-mcp` runs, without a server:
  - parser and rendering tests (`tests/sparql.rs`);
  - property tests (`tests/composition_props.rs`).
- The property tests (1,024 cases each by default) generate random queries: triples, `GRAPH`,
  `OPTIONAL`, `UNION`, `FILTER`, `BIND`, `MINUS`, nested groups and property-function calls, over random data in several named graphs and the default
  graph. They check that:
  - the query the server sends (functions pre-evaluated, dataset clause added) gives the same answers
    as a reference query built independently and evaluated by Oxigraph's `spareval` over a merged
    default graph;
  - printing a parsed query back out (`sparql::to_sparql`) keeps its meaning;
  - every local call site is replaced and calls inside `SERVICE` are left alone;
  - a dataset clause changes nothing but the dataset.

  Counterexamples found are kept in `tests/composition_props.proptest-regressions` and replayed
  first on every run. For a deeper run: `PROPTEST_CASES=50000 cargo test --release -p rdf-mcp --test
  composition_props`.

  Planted bugs are caught: `FROM NAMED` limited to the selection, a dropped `VALUES` table, and
  leftover list triples each produce a shrunk counterexample.
- With the local stack running and seeded, `just mcp-test` runs:
  - `tests/live.rs`, which drives every tool over MCP;
  - `tests/composition.rs`, which runs 11 worked queries that each combine several plugins and must
    pass a clean dry run.

  The plugins combined include search with inference, the catalog with inference, custom properties,
  the virtual graph with local graphs, OWL 2 QL reasoning in Ontop, search with federation, SQL with
  another dataset, a function evaluated at a `SERVICE` endpoint, a SHACL report with its data, and
  CONSTRUCT.
