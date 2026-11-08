# Custom SPARQL property functions

This plugin adds virtual predicates that **generate rows** inside a SPARQL query.
It is the extension point for future search plugins returning document/score
bindings. The current examples perform string operations; they do not implement
vector, semantic, or BM25 search.

The default `rdftool` application registers these properties for database, RDF-file,
and HTTP queries. In Studio, select **Custom property · tokens** and run the query.
No RDF upload is needed for the examples.

```sparql
PREFIX property: <urn:rdftool:property:>

SELECT ?token ?position
WHERE {
  (?token ?position) property:tokens "hello semantic world" .
}
ORDER BY ?position
```

Expected rows: `("hello", 0)`, `("semantic", 1)`, `("world", 2)`.
Positions are `xsd:integer`; tokens are `xsd:string`. Whitespace splitting uses
Unicode whitespace; an empty/whitespace-only input produces zero rows.

```sparql
PREFIX property: <urn:rdftool:property:>

SELECT ?word
WHERE { ?word property:split ("red,blue,red" ",") }
ORDER BY ?word
```

This returns `blue`, `red`, `red`. Duplicates and empty split fields are preserved.
The separator must be nonempty. Both properties require `xsd:string` inputs;
language-tagged strings and numeric literals are rejected rather than coerced.
Result ordering is unspecified unless the query includes `ORDER BY`.

Ordinary RDF joins work as usual:

```sparql
PREFIX property: <urn:rdftool:property:>
SELECT ?document ?word
WHERE {
  ?document <https://example.org/label> ?word .
  ?word property:split ("red,blue" ",") .
}
```

## Defining your own property

Implement `rdf_plugin_api::PropertyFunction`, which has only two methods:

```rust
use anyhow::{Result, ensure};
use rdf_plugin_api::{PropertyDescriptor, PropertyFunction, ReadContext, RdfValue, RowSink};

struct Identity;
impl PropertyFunction for Identity {
    fn descriptor(&self) -> PropertyDescriptor {
        PropertyDescriptor {
            iri: "https://example.org/property/identity".into(),
            description: "Return the input RDF value unchanged".into(),
            input_arity: 1,
            output_arity: 1,
        }
    }
    fn evaluate(&self, _: &dyn ReadContext, input: &[RdfValue], out: &mut dyn RowSink) -> Result<()> {
        ensure!(input.len() == 1, "identity expects one input");
        out.emit(vec![input[0].clone()])
    }
}
```

Register it in the application composition root, before sharing the database:

```rust
database.register(
    rdf_plugin_custom_properties::CustomProperties::examples().with_property(Identity)
)?;
```

Then query `?value <https://example.org/property/identity> "hello"`.
Alternatively, a separate plugin crate can implement `Plugin::property_functions`
and return its own property implementations. No engine or backend implementation
changes are needed for a new property using this contract. Each IRI must be unique
within a database runtime. Registration is atomic; duplicate IRIs, invalid IRIs,
incompatible SDK versions and unsupported arities are rejected.

SDK v2 supports IRI and literal values, including datatypes and language tags.
Blank-node and RDF-star output values are not supported. Return each row through
`RowSink::emit` and propagate errors. Wrong output arity or invalid RDF values
fail the query. This is a Rust development API, not a facility to upload executable
code through HTTP. Native implementations remain trusted and unsandboxed.

## Execution contract and limits

- The subject is the output term, or a SPARQL collection of output terms. Outputs
  may be variables or constants; constants/repeated variables constrain matches.
- The object is a constant input, or a collection of constant inputs. Collection
  structure must be in the query's own basic graph pattern, finite and unshared.
  Stored RDF lists are not traversed.
- Input variables are unsupported, including variables bound by `VALUES`, `BIND`,
  another property, or an earlier triple. This release is **not** a correlated,
  per-input-binding Jena property-function implementation.
- Each occurrence executes once during query preparation, not once per outer row.
  Results become a bounded `VALUES` relation. Oxigraph evaluates joins, OPTIONAL,
  UNION, FILTER/EXISTS, aggregates, subqueries and output serialization.
- Predicates are virtual, not stored or inferred triples. Variable-predicate
  queries enumerate stored RDF only. A registered predicate used directly in a
  triple pattern selects its plugin relation rather than stored triples with that
  predicate. An unregistered `urn:rdftool:property:*` IRI is an error; other unknown
  predicates remain ordinary RDF predicates.
- Properties work in SELECT, ASK, CONSTRUCT and DESCRIBE query patterns. They are
  rejected inside GRAPH, queries with FROM/FROM NAMED, UPDATE WHERE clauses,
  compound property paths, and scalar expression calls. Their IRIs can still occur
  as ordinary RDF data or output template predicates; that does not invoke them.
- A property context can execute ordinary SELECT/ASK against the selected dataset.
  Context calls do not recursively execute properties. Each context call and the
  final query have separate snapshots. Plugins must be read-only; cross-call
  snapshot consistency for concurrent writes is not promised. Stateful search
  indexes require a further consistency/lifecycle design.
- Hard expansion checks: 64 property occurrences per query, up to 32 inputs and
  32 outputs per occurrence, 64 KiB input RDF payload per occurrence, 10,000 emitted
  rows and 8 MiB emitted RDF payload across the query. Rows rejected by constant
  constraints still count. These are expansion limits, not whole-process RAM limits.
- `LIMIT` does not reduce plugin expansion. Errors abort query preparation even
  inside OPTIONAL or an otherwise unneeded branch. No partial results are returned.
  CPU cancellation and isolation of native plugin execution are not provided.
- The entire query is validated before callbacks run. SERVICE/LOAD remain disabled.

Discovery: `GET /api/plugins` includes each property's IRI, description, input
arity and output arity. Calls go through ordinary `/query`; the backend contains
no plugin-specific handler. Plugin registration must be restored on application
startup; executable code is not persisted in database files or backups.

## Validation and references

```sh
cargo test -p rdf-engine --test properties
cargo test -p rdf-cli --test cli --test http
```

The browser smoke test also runs the Studio property template. Tests verify
expected rows and semantics; they are not a claim of full Jena compatibility.
Jena distinguishes [property functions that generate bindings](https://jena.apache.org/documentation/query/writing_propfuncs.html)
from scalar functions. Oxigraph supplies [scalar hooks](https://docs.rs/oxigraph/0.5.13/oxigraph/sparql/struct.SparqlEvaluator.html);
this implementation adds a deliberately bounded relation adapter using its parsed
SPARQL algebra and existing evaluator, without replacing the query engine.
