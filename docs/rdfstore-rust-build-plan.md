# RDF Store from Scratch in Rust — Phased Build Plan

> **Outdated in part (2026-11-08).** The repository took the plugin-and-sidecar route instead of building
> every subsystem in Rust (see ADR 0002 and ADR 0004). Where this plan disagrees, the plugin model wins:
> **Phase 16 (OBDA / R2RML) and the federation items in it (16.7) are rejected as written**, and
> Phase 15 (an in-Rust HNSW index) is superseded by the Qdrant semantic-search plugin.
> Virtual graphs and `SERVICE` federation now follow
> [federation-and-virtual-graphs-plan.md](federation-and-virtual-graphs-plan.md): Ontop for R2RML/SQL,
> Comunica for federation, both behind a plugin contract. The rest of this document is kept as history
> and background.

## Scope

Build a Rust-native RDF quad store with:

- W3C-compliant SPARQL 1.1 Query + Update
- RDF-star / SPARQL-star (first-class quoted triples)
- Classic `rdf:Statement` reification (falls out of plain triples)
- Named graphs with per-graph reasoning
- RDFS + Jena-style rule reasoning (forward + backward chaining)
- Vector embeddings with HNSW and SPARQL-accessible nearest-neighbour search
- HTTP SPARQL Protocol + Graph Store Protocol endpoints
- A unified CLI that can execute the same SPARQL query against either a `.ttl` file (transient) or the persistent DB — enabling automatic parity testing

Out of scope for v1: OWL 2 DL reasoning (tableaux), OBDA/R2RML rewriter (use Ontop as a separate service), SPARQL 1.1 Federation across remote endpoints.

---

## Architectural cornerstones (decide once, use everywhere)

### 1. Workspace layout

A Cargo workspace with crates split on clean semantic boundaries. This lets the agent compile and test slices in isolation, which matters a lot at AI velocity.

```
rdfstore/
├── Cargo.toml                  # workspace
├── crates/
│   ├── rdf-model/              # terms, Quad, dictionary trait
│   ├── rdf-io/                 # parsers + serializers (Turtle, N-Quads, …)
│   ├── rdf-store/              # storage traits + redb backend + indexes
│   ├── sparql-syntax/          # parser, AST, algebra IR
│   ├── sparql-eval/            # volcano executor + operators
│   ├── sparql-opt/             # statistics + join reordering
│   ├── rdf-reasoner/           # RDFS + rule engine
│   ├── rdf-vec/                # HNSW + embedding store
│   ├── rdf-server/             # axum HTTP endpoint
│   ├── rdf-cli/                # the CLI binary
│   └── rdf-testkit/            # W3C manifest runner, differential harness
└── tests/                      # top-level integration + W3C suite
```

### 2. The storage abstraction

Every phase that touches data uses the same trait, so indexes, memory stores, and different backends are swappable:

```rust
pub trait QuadStorage {
    type Tx<'a>: Transaction where Self: 'a;
    fn read_tx(&self) -> Self::Tx<'_>;
    fn write_tx(&mut self) -> Self::Tx<'_>;
}

pub trait Transaction {
    fn quads_matching(&self, pattern: QuadPattern) -> QuadIter<'_>;
    fn insert(&mut self, quad: EncodedQuad) -> Result<bool>;
    fn remove(&mut self, quad: EncodedQuad) -> Result<bool>;
    fn commit(self) -> Result<()>;
}
```

Provide three implementations in order of build: `MemQuadStorage` (BTreeMap-based, for tests), `RedbQuadStorage` (persistent, pure Rust), and optionally `RocksDbQuadStorage` later for performance work.

### 3. The CLI parity principle

Every CLI subcommand that reads data takes a `--source` that accepts:

- A file path (`.ttl`, `.nq`, `.trig`) → parse into an in-memory store, run, discard
- A database path (directory) → open persistent store, run
- A URL (`http://…`) → treat as a SPARQL endpoint

The *exact same* code path runs in all three cases because they all produce a `Box<dyn Transaction>`. Then:

```bash
# Basic query
rdftool query --source data.ttl "SELECT ?s WHERE { ?s a :Person }"
rdftool query --source ./mydb "SELECT ?s WHERE { ?s a :Person }"

# Parity / differential testing (built-in, critical for agent velocity)
rdftool diff --query q.sparql --sources data.ttl,./mydb,http://localhost:7878

# W3C suite
rdftool test-suite --manifest w3c/sparql11/manifest-all.ttl --source fresh
```

The `diff` subcommand is how the agent catches regressions without human review — any divergence across sources is a bug.

### 4. Testing doctrine

Four concentric rings, layered in from Phase 0:

1. **Unit tests** in every crate, co-located with code.
2. **Snapshot tests** with `insta` for parser output, algebra IR, query plans. Diffs are trivial for an agent to inspect.
3. **Property-based tests** with `proptest` for round-trips: parse→serialize→parse, encode→decode, query plan idempotency.
4. **Conformance tests** against the W3C RDF + SPARQL 1.1 test suites, run via the built-in manifest runner in `rdf-testkit`.
5. **Differential tests** vs. Oxigraph for every SPARQL feature. Oxigraph is the oracle until the agent passes enough of the W3C suite to be its own oracle.

### 5. Core dependency choices

| Concern | Primary | Notes |
|---|---|---|
| Parser combinators | `winnow` | Modern successor to `nom`; good error messages |
| Lexer | `logos` | Blazing-fast for SPARQL/Turtle tokenization |
| Persistent KV | `redb` | Pure Rust, ACID, MVCC — easy build, good for v1 |
| HTTP server | `axum` + `tokio` | Standard modern Rust async stack |
| CLI | `clap` v4 (derive) | Best-in-class |
| Errors | `thiserror` + `anyhow` | Libraries use `thiserror`, binaries use `anyhow` |
| Tracing | `tracing` + `tracing-subscriber` | Structured logs and spans |
| Snapshots | `insta` | `cargo insta review` is an agent's best friend |
| Property tests | `proptest` | Shrinks failures to minimal repros |
| Bench | `criterion` | Statistical regression detection |
| CLI assertions | `assert_cmd` + `predicates` | Integration tests for `rdftool` |
| Parallel test runner | `cargo-nextest` | Cuts feedback loop noticeably |
| HNSW | `hnsw_rs` or hand-rolled | Reference: Malkov & Yashunin 2016 |
| IRI parsing | `oxiri` | From the Oxigraph ecosystem, well-tested |
| UUIDs | `uuid` | For IRI slug minting |

Deliberately *not* recommending: `sled` (maintenance concerns), `pest` (slower, less expressive than winnow here), `hyper` directly (use axum), raw `lmdb` (C dependency; redb is enough).

---

## Phase 0 — Foundation

**Goal:** Empty but fully wired workspace. CI green. The CLI builds and runs `--help`.

**Tasks:**
1. Initialize workspace with the 10 crates listed above (even if empty).
2. Configure `Cargo.toml` with shared lints, MSRV, and `[workspace.dependencies]` for version pinning.
3. GitHub Actions (or local `just`/`make`) running: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo nextest run`, `cargo deny check`.
4. Scaffold `rdftool` binary with `clap` skeleton and subcommand stubs (`ingest`, `query`, `dump`, `diff`, `test-suite`, `serve`).
5. Set up `rdf-testkit` with a stub W3C manifest runner that can enumerate tests but reports `pending` for all.

**Exit criteria:** `cargo nextest run` passes on empty tests. CI pipeline runs end-to-end.

**References:** Oxigraph's workspace structure (`oxrdf`, `oxrdfio`, `oxttl`, `spargebra`, `sparopt`, `sparesults`) is the cleanest published reference — study the crate boundaries.

---

## Phase 1 — RDF Data Model

**Goal:** Core RDF terms as Rust types, plus the dictionary that encodes them as compact integer IDs.

**Deliverables in `rdf-model`:**
- `NamedNode`, `BlankNode`, `Literal` (with lang tag + datatype IRI), `Triple`, `Quad`, `GraphName`, `Subject`, `Predicate`, `Object`.
- `QuotedTriple` variant behind an `rdf-star` feature flag — build it in now to avoid retrofitting in Phase 12.
- `Term` enum spanning all of the above.
- IRI validation via `oxiri`.
- A `Dictionary` trait: `encode(term) -> TermId`, `decode(id) -> Term`, with an in-memory HashMap-backed implementation.
- Inline encoding scheme for small values: tag the top bits of a 64-bit `TermId` to distinguish between "dictionary lookup required" and "value inlined" (booleans, small ints, short strings ≤7 bytes). Huge performance win later; cheap to design in now.

**Verification:**
- Unit tests for each term type (construction, equality, display).
- `proptest` round-trip: arbitrary term → encode → decode → original.
- IRI parser tests against RFC 3987 edge cases.

**Exit criteria:** `cargo bench` shows term construction under 50ns, dictionary encode under 100ns for cache-hit cases.

**References:**
- RDF 1.1 Concepts spec
- Oxigraph's `oxrdf` crate
- Jena NodeIdInline scheme (read the Java source)
- Virtuoso IRI_ID encoding (published papers)

---

## Phase 2 — Parsers and Serializers

**Goal:** Round-trip all major RDF formats. This is the foundation for the CLI and for every conformance test.

**Build order (strictly):**
1. **N-Triples** — trivial line-based format. Start here to validate the data model and testing harness.
2. **N-Quads** — adds a 4th term for graph name. Also trivial.
3. **Turtle** — prefix handling, blank node scoping (`[]`, `()`), literals with quotes/long strings, numeric/boolean shorthand. Real parser work.
4. **TriG** — Turtle plus `GRAPH <iri> { … }` blocks.
5. **(Optional)** RDF/XML, JSON-LD. RDF/XML is painful; JSON-LD is huge. Defer unless needed.

**Tech:**
- Use `logos` for tokenization (it generates a fast DFA).
- Use `winnow` for the parser layer, producing a streaming iterator of `Quad`s.
- Streaming is required: the bulk loader must not load whole files into memory.

**Verification:**
- W3C RDF test suite: `rdf-tests/turtle/`, `rdf-tests/n-triples/`, `rdf-tests/n-quads/`, `rdf-tests/trig/`. Each has positive and negative syntax tests. Aim for 100% pass.
- Snapshot tests on parser output via `insta`.
- `proptest` round-trip: arbitrary graph → serialize Turtle → parse → equal (up to blank node renaming).
- Differential: parse with Oxigraph's `oxttl` and compare quad sets.

**Exit criteria:** Full W3C parser tests pass. Can parse a 100MB N-Triples file in streaming mode with <100MB resident memory.

**References:**
- W3C Turtle 1.1 spec (grammar in appendix)
- W3C RDF-star evaluation note (for quoted triple syntax `<< >>`)
- Oxigraph's `oxttl` crate

---

## Phase 3 — In-Memory Store + CLI v0.1

**Goal:** The CLI becomes useful. You can load a Turtle file and run basic quad-pattern queries.

**Deliverables:**
- `MemQuadStorage` in `rdf-store`: three BTreeMap indexes (SPOG, POSG, OSPG) over encoded IDs. In-memory dictionary.
- CLI subcommands working against `--source file.ttl`:
  - `ingest --from file.ttl --to file.nq` (format conversion)
  - `query-pattern --source file.ttl --subject ?x --predicate rdf:type` (BGP-of-one, pre-SPARQL)
  - `dump --source file.ttl --format nquads`
  - `stats --source file.ttl` (counts, distinct predicates, etc.)

**Verification:**
- Integration tests with `assert_cmd`: `rdftool query-pattern` on fixture files produces expected output.
- Snapshot test the CLI's table output.
- Benchmark loading 1M triples into memory.

**Exit criteria:** The CLI can replace `rapper -i turtle -o ntriples file.ttl` for supported formats.

---

## Phase 4 — Persistent Store

**Goal:** Same API as `MemQuadStorage`, backed by `redb`. The CLI now accepts `--source ./mydb`.

**Design:**
- **Dictionary persistence:** two tables — `term_to_id` (bytes → u64) and `id_to_term` (u64 → bytes). Use a monotonic counter table for next ID.
- **Quad indexes:** six permutations to cover all 16 triple-pattern combinations with one range scan each. Each index is a redb table with `(a, b, c, d)` big-endian concatenated keys and empty values.
  - `GSPO`, `GPOS`, `GOSP` — for graph-scoped patterns
  - `SPOG`, `POSG`, `OSPG` — for cross-graph patterns
- **Transactions:** redb already gives you MVCC and single-writer/multi-reader. Expose its read/write transactions through your `Transaction` trait.
- **Bulk loader:** disable index maintenance, sort externally (use `external-sort` crate or write merge-sort), write sorted batches. Target: ingest 10M triples in under a minute.

**Verification:**
- Every test that passed against `MemQuadStorage` must now pass against `RedbQuadStorage` — parameterize tests over the trait.
- Crash test: kill the process mid-transaction, reopen, verify invariants hold.
- Benchmark: single-quad insert, batch insert, pattern lookup latency.

**Exit criteria:** CLI parity achieved — identical results from `--source file.ttl` and `--source ./mydb` after ingesting the same file. Run `rdftool diff` to verify automatically.

**References:**
- RDF-3X paper (Neumann & Weikum 2008) for index design rationale
- HDT paper for dictionary compression ideas
- Jena TDB2 design notes on nodes-to-ids

---

## Phase 5 — SPARQL Parser and Algebra

**Goal:** SPARQL text → validated abstract syntax tree → algebra IR. No evaluation yet.

**Deliverables in `sparql-syntax`:**
- Lexer via `logos` covering SPARQL 1.1 tokens (keywords are case-insensitive — handle this in the lexer).
- Recursive-descent parser via `winnow` producing an AST that mirrors the SPARQL grammar.
- Algebra translation per §18 of the SPARQL 1.1 spec: `BGP`, `Join`, `LeftJoin`, `Union`, `Filter`, `Extend`, `Minus`, `Group`, `Aggregation`, `OrderBy`, `Project`, `Distinct`, `Reduced`, `Slice`, `ToList`, `Graph`, `Service` (stub).
- Algebra pretty-printer (for snapshot tests).
- Algebra serializer to SPARQL text (for round-trip verification).

**Verification:**
- **Snapshot tests:** parse each W3C syntax-test query, snapshot the algebra. Human (or agent) reviews the diffs once, then they become regression anchors.
- **Syntax tests:** W3C `syntax-query` and `syntax-fed`, including negative tests. Expect 100% on positive, 100% on negative (errors correctly rejected).
- **Round-trip property test:** parse → serialize → parse produces equal algebra.
- **Differential:** parse the same query with Oxigraph's `spargebra`, compare normalized algebra trees.

**Exit criteria:** All W3C SPARQL 1.1 syntax tests pass.

**References:**
- SPARQL 1.1 Query spec, especially §18 (Evaluation)
- Oxigraph's `spargebra` crate
- Jena's ARQ op model (study the `Op*` Java classes)

---

## Phase 6 — SPARQL Evaluator: SELECT Basics

**Goal:** `SELECT` queries work end-to-end with BGPs, FILTER, projection, DISTINCT, ORDER BY, LIMIT, OFFSET.

**Design:**
- **Volcano iterator model.** Every algebra operator becomes a struct implementing `Iterator<Item = Result<Solution>>`. Easy to compose, easy to debug, good enough performance for v1.
- **Solution = Vec<Option<TermId>>** indexed by variable ID. Avoid hashing variable names during evaluation.
- **BGP execution:** for now, simple left-deep nested-loop joins with index lookup. The optimizer phase comes later.
- **Expression evaluator** for FILTER: implement all XPath functions SPARQL requires (string, numeric, date, etc.). This is long but mechanical. Use `time` crate for dates.
- **Effective boolean value** rules per spec §17.2.

**Verification:**
- W3C SPARQL 1.1 query-evaluation tests: `basic/`, `expr-builtin/`, `expr-ops/`, `bind/`, `project-expression/`, `distinct/`, `cast/`. Target: 100% pass.
- Differential tests vs. Oxigraph on every query in the W3C suite.
- Property test: a query with `FILTER(true)` returns the same results as without the filter.

**Exit criteria:** The W3C basic evaluation tests pass. The CLI can execute a meaningful subset of SPARQL against `file.ttl` and `./mydb` with identical results.

---

## Phase 7 — SPARQL Evaluator: Full Query

**Goal:** Close the remaining SELECT features.

**In scope:**
- `OPTIONAL` (LeftJoin) — subtle interaction with FILTER: the FILTER inside OPTIONAL only rejects the optional match, doesn't drop the outer row. Most implementations get this wrong initially.
- `UNION`
- `MINUS`
- `GRAPH ?g { … }` — iterate over graph names (distinct subjects of `GSPO`) and re-evaluate inner pattern per graph.
- `BIND` (Extend) and `VALUES` (inline data)
- Subqueries (`{ SELECT … }`)
- `CONSTRUCT`, `DESCRIBE`, `ASK` query forms
- Result serialization: SPARQL-Results JSON, SPARQL-Results XML, CSV/TSV

**Verification:**
- W3C `optional/`, `optional-filter/`, `negation/`, `subquery/`, `bindings/`. Target: 100% pass.
- Result-format tests: W3C SPARQL Results tests.

**References:**
- SPARQL 1.1 spec §18.5 (evaluation semantics) — read it three times
- Cyganiak "A Relational Algebra for SPARQL"

---

## Phase 8 — SPARQL Evaluator: Aggregates + Property Paths

**Goal:** Two features with very different implementation profiles.

**Aggregates:** `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `SAMPLE`, `GROUP_CONCAT`, with `GROUP BY`, `HAVING`, and `DISTINCT` modifiers. Build a hash-based group operator.

**Property paths:** `p/q`, `p|q`, `p*`, `p+`, `p?`, `^p`, `!p`, `!(p|q)`. Fixed paths (`/`, `|`, `?`) translate to algebra rewrites. Variable-length paths (`*`, `+`) require graph traversal — BFS with a visited set per starting node. Keep a separate `PropertyPath` operator.

**Verification:**
- W3C `aggregates/`, `property-path/`.
- Stress test: `?s (rdfs:subClassOf)* ?o` over a 10,000-class hierarchy — must terminate and not duplicate results.

---

## Phase 9 — SPARQL UPDATE

**Goal:** `INSERT`, `DELETE`, `INSERT DATA`, `DELETE DATA`, `INSERT WHERE`, `DELETE WHERE`, `LOAD`, `CLEAR`, `CREATE`, `DROP`, `COPY`, `MOVE`, `ADD`.

Each update is executed in a single write transaction. Use the storage abstraction's `Transaction::insert` / `::remove`.

**Verification:** W3C SPARQL 1.1 Update tests. Transaction tests: interleaved reads see consistent snapshots during a long update.

---

## Phase 10 — Query Optimizer

**Goal:** Turn the naive left-deep plan into something reasonable.

**Build order:**
1. **Statistics collection:** per-predicate counts, per-graph counts, distinct subject/object counts per predicate. Stored in a system graph or sidecar.
2. **Cardinality estimation** for each triple pattern, using statistics and pattern shape.
3. **Join reordering:** start with greedy (pick smallest estimated join at each step). Upgrade to DP-based (IKKBZ or Selinger-style) once the infrastructure is solid.
4. **Filter pushdown:** move FILTERs as close to their source as possible.
5. **Sideways information passing:** propagate bound variable values across joins early.
6. **Projection pushdown:** drop unused variables early to shrink intermediate solutions.

**Verification:**
- Benchmark suite: BSBM, LUBM, WatDiv — all public SPARQL benchmarks. Track query latency before/after each optimizer change.
- Differential against Oxigraph on result correctness (not perf).
- Snapshot tests on the chosen plan for a representative query set.

**References:**
- Neumann & Weikum RDF-3X papers (join ordering with bushy plans)
- Stocker et al. "SPARQL basic graph pattern optimization using selectivity estimation"
- DuckDB's and Umbra's optimizer blog posts for general ideas

---

## Phase 11 — HTTP SPARQL Protocol

**Goal:** `rdftool serve --db ./mydb --bind 0.0.0.0:7878` exposes a real SPARQL endpoint.

**Endpoints:**
- `GET/POST /query` — SPARQL 1.1 Protocol, content negotiation for JSON/XML/CSV/TSV/Turtle
- `POST /update` — SPARQL 1.1 Update
- `GET/PUT/POST/DELETE /store?graph=<iri>` — Graph Store Protocol
- `GET /health`, `GET /metrics` (Prometheus)

Use `axum`. Authentication and CORS configurable via flags.

**Verification:**
- Run the W3C SPARQL 1.1 Protocol test suite against the running server.
- The CLI's `--source http://localhost:7878` mode now works, giving three-way parity (file / DB / HTTP).

---

## Phase 12 — RDF-star and SPARQL-star

**Goal:** First-class quoted triples end-to-end.

Since Phase 1 already modeled `QuotedTriple` as a Term variant, this phase is about wiring:

- **Dictionary:** quoted triples get their own ID space. Encode as `(subject_id, predicate_id, object_id)` tuples hashed into a dedicated table.
- **Parsers:** enable `<< s p o >>` syntax in Turtle-star, TriG-star, N-Triples-star, N-Quads-star.
- **SPARQL-star:** extend parser to accept `<< >>` as a term position and as a binding target.
- **Indexes:** quoted triples must be findable. A quoted triple is just a term, so it already goes into the regular indexes when it appears as subject or object. The *inner* triples of a quoted triple need a secondary index if you want to query them — add a `QuotedTripleIndex` mapping inner (s, p, o) → quoted-triple ID.
- **Annotation syntax:** `<< s p o >> :assertedBy :alice .` desugars to two triples.

**Verification:**
- W3C RDF-star test suite (once stable) and the RDF 1.2 CG test suite.
- Differential against Oxigraph's rdf-12 feature.

**References:**
- Hartig's RDF-star foundational papers
- RDF 1.2 working drafts
- Oxigraph's RDF-star implementation

---

## Phase 13 — RDFS Reasoning

**Goal:** Closure under the RDFS entailment rules — either fully materialized ahead of time, or lazily during query evaluation.

**Implementation — forward materialization (simpler first):**
- A `Reasoner` struct that takes a graph and produces additional inferred quads written to a dedicated `inferred` named graph.
- Rules: `rdfs2` (domain), `rdfs3` (range), `rdfs5` (subPropertyOf transitive), `rdfs7` (subPropertyOf application), `rdfs9` (subClassOf application), `rdfs11` (subClassOf transitive). The full set is 13 rules.
- Semi-naïve evaluation: in each iteration, only apply rules to newly inferred triples from the last iteration. Fixpoint when no new triples.

**Verification:**
- W3C RDF Semantics entailment test suite.
- The agent can `rdftool reason --db ./mydb --rules rdfs --output-graph :inferred` and verify inferred triples match expectations.

---

## Phase 14 — Rule Engine (Jena-style)

**Goal:** User-defined rules in Jena Rule Language syntax, supporting forward and backward chaining.

**Design:**
- Rules are stored as RDF literals in a dedicated named graph (aligns with your existing architecture).
- Parser for Jena Rule syntax: `[ruleName: (s p o) (s p o) -> (s p o)]`.
- **Forward chaining:** Rete or semi-naïve Datalog evaluator. Start with semi-naïve — simpler, adequate for most workloads.
- **Backward chaining:** SLD resolution. Integrate into the SPARQL evaluator as a virtual source for matching triple patterns.
- **Chaining mode** per rule, chosen by a hint or by static analysis (recursive rules → forward; rules with large RHS → backward).

**Verification:**
- Reimplement Jena's `rdfs-fb.rules` and verify identical closure to Phase 13.
- OWL 2 RL rule set (published as Datalog rules) — a good stress test.

**References:**
- Abiteboul, Hull, Vianu *Foundations of Databases* (ch. 12–15 on Datalog)
- Jena Inference documentation
- Motik et al. "OWL 2 Web Ontology Language Profiles" §4.3 (OWL 2 RL rules)
- RDFox paper (state of the art in Datalog-based RDF reasoning)

---

## Phase 15 — Vector Embeddings and Search

> **Superseded.** Implemented as the Qdrant-backed `semantic-search` plugin (ADR 0004, ADR 0009) instead of an in-Rust HNSW index.

**Goal:** Attach dense vectors to IRIs, query them from SPARQL.

**Storage:**
- A sidecar index in the same `redb` database: `EmbeddingStore { iri_id → Vec<f32> }`.
- An HNSW index over the vector space, persisted to disk. Use `hnsw_rs` or implement from the Malkov & Yashunin paper.

**SPARQL surface:**
Expose nearest-neighbour as a magic property function:

```sparql
PREFIX vec: <http://example.org/ns/vec#>

SELECT ?doc ?score WHERE {
  (?doc ?score) vec:nearestTo (<http://ex/query-entity> 10) .
  ?doc a :Document .
}
```

The algebra translator recognizes `vec:nearestTo` and produces a `VectorSearch` operator feeding the BGP join.

**CLI:**
- `rdftool embed --source file.ttl --model sentence-transformers/all-MiniLM-L6-v2 --target ./mydb`
- `rdftool search --db ./mydb --query "machine learning" --top-k 10`

**Verification:**
- Recall@k benchmarks on standard IR datasets (MS MARCO subset).
- Property test: exact search (brute force) and HNSW return top-k with ≥95% overlap.
- Persistence test: reopen DB, search still works.

**References:**
- Malkov & Yashunin 2016 HNSW paper
- FAISS documentation
- Qdrant / Weaviate architecture posts

---

## Phase 16 — OBDA / Virtual Knowledge Graph (SPARQL-to-SQL)

> **REJECTED — outdated.** Do not build an R2RML parser, unfolder, SQL optimizer, dialect executors, OWL 2 QL
> rewriter or native-store federation in Rust. That is a multi-month rebuild of Ontop. The adopted approach is
> to run Ontop as a sidecar and register it as an approved SPARQL service, per
> [federation-and-virtual-graphs-plan.md](federation-and-virtual-graphs-plan.md), so the feature ships quickly
> through the plugin model. The text below is retained only as background on what Ontop does.

**Goal:** SPARQL queries resolve transparently against relational sources through R2RML mappings, producing a single SQL query executed on the remote database. A SPARQL `SELECT` over `:Customer` translates into a BigQuery SQL SELECT over `analytics.customers`, threaded through the ontology at query time, with zero ETL.

This is the largest single phase in the plan — a full OBDA engine is what Ontop exists to be, and rebuilding it is a multi-month effort even at agent velocity. Commit to it deliberately or embed Ontop instead.

**Scope in v1:**
- R2RML mapping support. Mappings are serialized as Turtle, so the RDF parser from Phase 2 loads them for free — the "TTL format" for semantic binding is literally just R2RML.
- Single-source virtualization per query branch (one SQL database on one side of a join).
- OWL 2 QL rewriting for TBox-aware queries (subClassOf / subPropertyOf / inverse expansion). QL is the *only* OWL profile that makes virtual access tractable — RL requires materialization, EL is not relational-friendly.
- BigQuery SQL dialect as primary target (matches your existing stack); Postgres as the second dialect for local, hermetic integration testing.
- Federation between the native quad store and virtual sources in a single query.

**Out of scope for v1:**
- RML extensions (JSON, CSV, XML sources) — add as Phase 16.8 later.
- OWL 2 RL/EL rewriting through virtual sources.
- R2RML-star (quoted triples through mappings).
- Bidirectional updates through mappings.
- Cross-database SQL federation (one query, multiple SQL sources).

### Architecture

Introduce a new abstraction in `rdf-store`:

```rust
pub trait VirtualSource {
    fn mappings(&self) -> &MappingSet;
    fn rewrite(&self, algebra: &Algebra, tbox: &TBox) -> Result<SqlPlan>;
    fn execute(&self, plan: &SqlPlan) -> BoxStream<'_, Result<Solution>>;
    fn dialect(&self) -> &dyn SqlDialect;
}
```

The evaluator, on encountering a subtree whose triple patterns all resolve through mappings of a single `VirtualSource`, delegates the whole subtree to that source. The `SqlPlan` is executed remotely; rows come back and are reconstructed into SPARQL solutions using the mappings' IRI templates.

### Implementation pieces, in order

**1. Mapping parser and model.** R2RML is just RDF, so ingestion is free. Build an assertion layer that turns R2RML RDF into typed structs: `TriplesMap`, `LogicalTable` (base table or R2RML view with arbitrary SQL), `SubjectMap`, `PredicateObjectMap`, `TermMap` (template / column / constant), `JoinCondition`, `RefObjectMap`. Validate at load time — broken mappings fail fast, not at query time.

**2. Mapping index.** Precompute, for every predicate and for every class IRI, which `TriplesMap`s can produce matching triples. This is what makes unfolding fast enough to run per-query.

**3. Unfolder.** The heart of the rewriter. Each SPARQL triple pattern `?s :p ?o` gets replaced with the union of SQL subqueries from every mapping that could produce triples matching `:p`. Variables become SQL column projections wrapped in IRI-template reconstruction expressions.

**4. OWL 2 QL rewriter.** Implement PerfectRef (Calvanese et al.) or its faster descendant Presto. Given a query and a QL TBox, expand each triple pattern to cover subclasses, subproperties, and inverses. `?s a :Employee` becomes a union over `:Employee` and every subclass, each unfolded through mappings independently. This is where OBDA earns its reputation for explosive SQL — good rewriters keep it tractable via query containment checks and T-mapping precompilation.

**5. SQL AST and generator.** Hand-rolled SQL AST, not a parser. Nodes: `SqlSelect`, `SqlJoin`, `SqlUnion`, `SqlFilter`, `SqlProject`, `SqlSubquery`, plus expression nodes. A separate pretty-printer per dialect translates the AST to a dialect-specific string. Do not use `sqlparser-rs` — you're generating SQL, not parsing it, and owning the AST end-to-end matters for the optimizer.

**6. SQL optimizer.** The naive unfolded SQL is catastrophic — dozens of unions, redundant self-joins, IRI strings concatenated for join keys that can never match. You need, at minimum:
- **Join elimination** via foreign-key reasoning: if a mapping declares an FK and only parent columns are referenced, drop the child join.
- **Self-join elimination** when the same logical table appears multiple times with compatible conditions.
- **Union pruning** via IRI template containment: two unions whose IRI templates can never produce overlapping values get pruned on unsatisfiable join conditions.
- **Constant propagation and filter pushdown.**
- **Projection pushdown** to minimize row width early.

These are the rewrites that turn a 400-line unfolded monster into a 40-line tractable query. Ontop's optimizer is the reference; papers by Kontchakov, Rodriguez-Muro, and Xiao document each rewrite individually.

**7. Dialect layer.** A `SqlDialect` trait: identifier quoting, function-name mapping, date/time semantics, string-concat operator, IRI template materialization (`CONCAT` vs `||`), NULL ordering. BigQuery and Postgres as the two initial implementations. Each dialect gets its own golden-SQL snapshot suite.

**8. Executor.** For BigQuery, use the REST API via `reqwest` with service-account auth, or the `gcp-bigquery-client` crate. For Postgres, use `sqlx`. Stream rows, do not materialize. Convert each row to a `Solution` using the mapping's IRI templates and datatype declarations.

**9. Federation with the native store.** When a query spans virtual and native patterns, the planner splits the algebra tree at the boundary. The standard technique: evaluate one side, materialize the set of join-key IRIs, inject them as a `VALUES` clause into the SQL sent to the virtual source. This is structurally the same pattern you already use for `owl:sameAs` cluster expansion — same operator, different source on each side. Cap the injected set size and fall back to full-scan-then-hash-join above a threshold.

### Failure modes to design for

- **Unbounded BGPs** (`?s ?p ?o`) against virtual sources — forces a union across every mapping. Either forbid with a clear error or cap the mapping count and emit a warning in `EXPLAIN`.
- **Property paths on virtual data** — `rdfs:subClassOf*` has no SQL equivalent unless the hierarchy is materialized. Detect and either materialize the relevant hierarchy at plan time (into a native graph) or reject the query.
- **Aggregates crossing the native/virtual boundary** — forces full materialization of one side. Make it explicit in `EXPLAIN`, never silent.
- **Nested OPTIONALs over virtual data** — generate correct SQL LEFT JOINs, but surface the plan complexity clearly.

### CLI additions

```bash
# Validate mappings
rdftool r2rml validate --mappings mappings.ttl

# Explain the SQL a SPARQL query produces, without executing
rdftool vq explain --mappings mappings.ttl --tbox ontology.ttl \
  --query "SELECT ?c WHERE { ?c a :Customer }" --dialect bigquery

# Unfold a single pattern, show the intermediate
rdftool r2rml unfold --mappings mappings.ttl --pattern "?s a :Customer"

# Execute against a virtual source
rdftool query --mappings mappings.ttl --source bigquery://project/dataset \
  "SELECT ?c ?name WHERE { ?c a :Customer ; :name ?name } LIMIT 10"

# Parity check: materialize a bounded slice to TTL, then diff against virtual
rdftool vq materialize --mappings mappings.ttl --source bigquery://... \
  --limit-per-class 1000 --output sample.ttl
rdftool diff --query q.sparql --sources sample.ttl,bigquery://...,native-db
```

The `vq materialize --limit` + `diff` workflow is the keystone verification loop for this phase: pull a bounded sample through the mappings into TTL, load it into the native store, run the same SPARQL against both, confirm identical results. Any divergence is a rewriter bug. This is how the agent self-verifies without needing a human to read SQL.

### Verification

- **R2RML W3C test cases** — structural correctness of mapping interpretation.
- **Ontop's published conformance suite** — the de facto OBDA test oracle.
- **NPD benchmark** (Norwegian Petroleum Directorate) — the canonical OBDA benchmark, with published queries, expected cardinalities, and realistic mapping complexity.
- **Differential vs. materialized**: for each mapping set, `vq materialize` the full (or a bounded sample of the) virtual KG into the native store, then run every SPARQL test query against both. Results must match (modulo blank-node renaming).
- **Golden SQL snapshots**: a fixed query corpus × dialect matrix, snapshot the generated SQL with `insta`. The agent reviews diffs on every rewriter change.
- **Hermetic integration tests** against a Postgres Docker container with a known schema and mappings — fast, runnable in CI, catches dialect bugs early.
- **BigQuery live tests**, gated on credentials, run nightly against a small fixture dataset.

### Realistic sizing

Even at agent velocity, this phase decomposes into sub-phases that should each be their own PR series:

- 16.1: R2RML parser and mapping model
- 16.2: Unfolder (no ontology rewriting)
- 16.3: SQL AST + Postgres dialect + Postgres executor
- 16.4: SQL optimizer (join elim, union pruning, FK reasoning)
- 16.5: BigQuery dialect + executor
- 16.6: OWL 2 QL rewriter (PerfectRef / Presto)
- 16.7: Federation with the native store

Do not conflate them. Each has its own verification surface and its own class of bugs.

### References

**Foundational:**
- Poggi et al. 2008, "Linking Data to Ontologies" — the OBDA formalism.
- Calvanese et al., "Ontologies and Databases: The DL-Lite Approach."
- Xiao et al. 2018, "Ontology-Based Data Access: A Survey" (IJCAI). Start here.

**Rewriting algorithms:**
- Calvanese, De Giacomo, Lembo, Lenzerini, Rosati — PerfectRef.
- Rodriguez-Muro and Calvanese — T-Mappings (TBox precompilation).
- Kontchakov et al. on rewriting optimization.
- Bursztyn, Goasdoué, Manolescu — Presto (a faster rewriter).

**Engine design:**
- Ontop papers by Calvanese, Xiao, Lanti, Rezk, Kontchakov.
- Ontop source code (Java, clean architecture) — the single best reference implementation. Read `ontop-mapping-sql` and `ontop-system-core`.
- Morph-RDB — an earlier, simpler R2RML engine, easier to read for the basics.
- Mastro — academic OBDA system predating Ontop.

**Standards:**
- W3C R2RML spec (2012).
- W3C Direct Mapping spec.
- RML.io materials for the multi-format extension (future phase).
- Motik et al., OWL 2 Profiles spec — QL section.

---

## Phase 17 — OWL 2 RL (Optional but High-Value)

**Goal:** Subset of OWL 2 RL via the rule engine from Phase 14.

OWL 2 RL is fully expressible as Datalog rules (the spec publishes them). Load them into the rule engine. Verify with the OWL 2 RL conformance tests.

This phase is optional for v1 — your primary ontology work happens in Protégé and you can materialize once.

---

## Phase 18 — Hardening

**Not a single phase, but a checklist to close before calling it done:**

- **Bulk loader** with parallel parsing and sorted batch writes. Target: 100k triples/sec sustained.
- **Backup/restore** — leverage redb's snapshot capabilities.
- **Metrics** — Prometheus endpoint for query counts, latencies, cache hit rates.
- **Structured query logging** with sampling.
- **Configuration** — TOML config file with env-var overrides.
- **Documentation** — rustdoc on every public item, an mdbook architecture guide, and example projects.
- **Fuzzing** — `cargo-fuzz` harnesses on the parser and the SPARQL evaluator. Run for 24+ CPU-hours.

---

## How the agent verifies quality, continuously

The agent should have five always-on feedback channels, plus one periodic:

1. **`cargo clippy -- -D warnings`** and `cargo fmt --check` block every PR.
2. **`cargo nextest run`** runs the full unit + integration suite. Target: under 60 seconds for incremental, under 5 minutes for full.
3. **`cargo insta test`** gates snapshot drift. `cargo insta review` is how the agent inspects diffs.
4. **W3C conformance runner** runs against `manifest-all.ttl` on every PR that touches parsing, algebra, or evaluation. Pass rate is tracked as a metric over time — it should monotonically increase.
5. **Differential suite** against Oxigraph for every query in a fixed query corpus (build this corpus by extracting queries from the W3C suite plus public benchmark queries). Any divergence is a blocker.
6. **Nightly fuzzing + criterion benchmarks** with regression alerts on >5% perf drops.

The single best pattern is the `rdftool diff` command in the CLI: it runs the same query against all configured sources (file, local DB, Oxigraph HTTP, reference Jena HTTP) and reports any result divergence. Have the agent call it reflexively whenever it touches evaluation code.

---

## Reference reading, prioritized

**Must read before starting:**
- W3C RDF 1.1 Concepts and Abstract Syntax
- W3C SPARQL 1.1 Query Language (especially §17–§18)
- W3C SPARQL 1.1 Update
- SPARQL 1.1 Protocol + Graph Store HTTP Protocol
- Neumann & Weikum 2008 — "RDF-3X: A RISC-Style Engine for RDF" (the index design canon)
- Hartig "Foundations of RDF★ and SPARQL★"

**Study source code:**
- **Oxigraph** — the Rust reference. Read `oxrdf`, `oxttl`, `spargebra`, `sparopt`, `sparesults` in that order.
- **Apache Jena** — the semantics reference. ARQ for SPARQL, TDB2 for storage, Inference engine for rules.
- **RDFox** (academic papers) — state-of-the-art Datalog reasoning and parallel materialization.
- **HDT** — compact read-only RDF format. Great dictionary encoding ideas.

**Background:**
- Abiteboul, Hull, Vianu *Foundations of Databases*
- Hogan *The Web of Data* (Springer, free online)
- Cyganiak "A Relational Algebra for SPARQL" (HP Labs TR)

**Benchmarks to run:**
- BSBM (Berlin SPARQL Benchmark)
- LUBM (Lehigh University Benchmark, for reasoning)
- WatDiv (query diversity)
- The W3C SPARQL 1.1 test suite (correctness, not speed)

---

## Recommended execution order summary

The dependency graph between phases is mostly linear with a few parallelizable branches:

```
0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 11
                              ↓
                             10 (can start after 6, polish after 8)
                              ↓
                             12 (RDF-star, can start anytime after 1)
                              ↓
                             13 → 14 → 17 (reasoning chain, OWL 2 RL)
                              ↓
                             15 (vectors, independent of reasoning)
                              ↓
                             16 (OBDA — depends on 5, 7, 10; benefits from 14)
                              ↓
                             18 (hardening, always-on)
```

Phase 10 (optimizer), Phase 12 (RDF-star wiring), Phase 15 (vectors), and Phase 16 (OBDA) can run as parallel threads of work once Phases 0–7 are solid. Phase 16 is the heaviest of the parallel threads and should own its own long-running branch. Everything downstream of Phase 4 depends on the storage abstraction being stable, so don't skimp there.

The single most important early investment is the CLI's `--source` abstraction and the differential `diff` subcommand. Every subsequent phase becomes self-verifying because of it.
