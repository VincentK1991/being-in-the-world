# Apache Jena CLI Command Reference

## Why this exists
When implementing RDF store features, we can use Apache Jena CLI commands as an independent check for parser, query, update, and storage behavior.

This reference is organized to support differential testing and regression checks.

## Baseline setup
- Install Apache Jena binary distribution.
- Set `JENA_HOME` and add `$JENA_HOME/bin` to `PATH` (or `%JENA_HOME%\\bat` on Windows).
- Use `--help` on every command for exact flags in your installed version.

## Command inventory (31 top-level commands)

### RDF parse/convert tools (7)
| Command | What it does | Independent check usage |
|---|---|---|
| `riot` | General RDF parser/converter/validator across RDF syntaxes. | Validate our parser acceptance/rejection and serialization round-trips. |
| `turtle` | `riot` specialization for Turtle input. | Turtle syntax conformance checks. |
| `ntriples` | `riot` specialization for N-Triples input. | N-Triples parser parity. |
| `nquads` | `riot` specialization for N-Quads input. | Dataset/quad parser parity. |
| `trig` | `riot` specialization for TriG input. | Named graph parse parity. |
| `rdfxml` | `riot` specialization for RDF/XML input. | RDF/XML ingest checks (if/when supported). |
| `rdfparse` | RDF/XML parser utility (including embedded `rdf:RDF` extraction use cases). | Edge-case RDF/XML diagnostics. |

### SPARQL query/update tools (6)
| Command | What it does | Independent check usage |
|---|---|---|
| `arq` | SPARQL query driver wrapper. | Compare query result sets on local files. |
| `sparql` | SPARQL query wrapper command. | Easy baseline query runner for parity tests. |
| `qparse` | Parses and pretty-prints SPARQL queries. | Syntax acceptance/rejection parity for query parser work. |
| `uparse` | Parses and pretty-prints SPARQL Update requests. | Update parser conformance checks. |
| `rsparql` | Sends query to remote SPARQL endpoint. | Compare local engine results against Jena/Fuseki endpoint behavior. |
| `rupdate` | Sends update to remote SPARQL endpoint. | Update semantics parity via remote endpoint. |

### TDB/TDB2/Fuseki dataset tools (13)
| Command | What it does | Independent check usage |
|---|---|---|
| `tdbquery` | Query TDB1 dataset. | Legacy store baseline comparisons. |
| `tdb2.tdbquery` | Query TDB2 dataset. | Primary persistent query baseline. |
| `tdbdump` | Dump TDB1 dataset (N-Quads output). | Export/normalization parity checks. |
| `tdb2.tdbdump` | Dump TDB2 dataset. | Persistent dataset verification and diffs. |
| `tdbupdate` | Execute update against TDB1 dataset. | Update semantics baseline for TDB1 workflows. |
| `tdb2.tdbupdate` | Execute update against TDB2 dataset. | Primary update parity for persistent datasets. |
| `tdbloader` | Load data files into TDB1 dataset. | Ingest behavior and load-pipeline comparisons. |
| `tdb2.tdbloader` | Load data files into TDB2 dataset. | Primary ingest baseline for persistent dataset tests. |
| `tdbstats` | Produce dataset statistics (TDB1). | Query planning/statistics sanity checks. |
| `tdb2.tdbstats` | Produce dataset statistics (TDB2). | Optimizer baseline and stats generation checks. |
| `tdbbackup` | Create gzipped backup of TDB1 dataset triples. | Backup/export operational parity. |
| `tdb2.tdbbackup` | Create backup of TDB2 dataset. | Backup verification for persistent store features. |
| `tdb2.tdbcompact` | Compact TDB2 dataset storage. | Storage maintenance feature parity and operational tests. |

### Validation and utility tools (4)
| Command | What it does | Independent check usage |
|---|---|---|
| `shacl` | Validates RDF against SHACL shapes. | Future feature checks for shape validation support. |
| `shex` | Validates RDF using Shape Expressions (ShEx). | External validation oracle in pipeline checks. |
| `rdfdiff` | Compares RDF datasets/graphs independent of serialization. | Golden diff tool for regression and parity testing. |
| `iri` | Parses/validates IRIs and reports warnings/errors. | IRI validation behavior cross-check for `rdf-model`. |

### Ontology codegen tool (1)
| Command | What it does | Independent check usage |
|---|---|---|
| `schemagen` | Generates Java vocabulary constants from OWL/RDFS. | Useful reference for ontology vocabulary extraction workflows. |

## ARQ lower-level command family (supplement)
Jena ARQ docs also describe command names often invoked directly in scripts:
- `arq.query`
- `arq.qparse`
- `arq.uparse`
- `arq.update`
- `arq.remote`
- `arq.rset` (result set transformations)
- `arq.qexpr` (expression evaluation)

Treat these as lower-level command entry points behind/wrapping higher-level scripts.

## Independent-check playbook for this repo

### 1) Parser conformance
- Run our parser on fixture corpus.
- Run `riot --validate` on same corpus.
- Diff accepted/rejected sets and normalize expected exceptions.

### 2) Query parity
- Load identical dataset into Jena (file-based or TDB2).
- Execute query with `sparql`/`arq` and with our engine.
- Normalize result order and formats, then compare.

### 3) Update parity
- Apply updates with `tdb2.tdbupdate` (or `rupdate` to endpoint).
- Apply same updates in our store.
- Compare final datasets via canonical N-Quads and `rdfdiff`.

### 4) Persistent store ingest/export checks
- Load source corpus with `tdb2.tdbloader`.
- Dump with `tdb2.tdbdump`.
- Compare against our store dump to catch ingest/indexing regressions.

### 5) IRI and syntax edge cases
- Validate problematic IRIs with `iri`.
- Validate parse edge cases with `qparse`/`uparse` and `riot`.
- Use differences to build targeted regression tests.

## Important compatibility note
- TDB1 and TDB2 are not compatible.
- Do not run TDB1 tools on TDB2 databases or vice versa.

## Sources
- https://jena.apache.org/documentation/tools/
- https://jena.apache.org/documentation/query/cmds.html
- https://jena.apache.org/documentation/tdb2/tdb2_cmds.html
- https://jena.apache.org/documentation/tdb/commands.html
- https://jena.apache.org/documentation/tools/schemagen.html

