# SHACL validation plugin

Validate RDF against ordinary RDF shapes using Apache Jena 6.2.0 SHACL Core.
This is independent of the inference plugin: inference derives facts; validation
reports whether the selected data satisfies constraints.

## Useful policies

| Behavior | Useful for | Available |
| --- | --- | --- |
| Report-only | Exploring imported data; quality dashboards; legacy datasets | `validate`, default workflow |
| Advisory warnings | Missing descriptions, recommended identifiers, enrichment opportunities | `sh:Warning`/`sh:Info` in shapes; findings shown in Studio |
| Explicit gate | CI checks and import/promotion workflows that must stop on findings | `check`, nonzero CLI exit / HTTP 400 on threshold failure |
| Reject every invalid write | Enforcing database-wide invariants atomically | Future generic pre-commit host extension; not implemented |
| Quarantine then promote | Accept imperfect uploads into staging, review, then publish | Can manually validate staging; atomic promotion gate not implemented |

Violations, warnings, and information are **constraint severities**. Report-only
versus gating is **application policy**. Set `sh:severity` on each property shape;
omission means `sh:Violation`. Policy never rewrites the standard report's
`sh:conforms`: a warning-only nonconforming report may still pass your gate.

Gate thresholds in trusted configuration:

- `"fail_on": "violation"` (default): stop only for Violations.
- `"fail_on": "warning"`: stop for Warnings or Violations.
- `"fail_on": "info"`: stop for any finding.

A provider failure is an execution error, not a data-quality finding. Both
operations fail on execution errors and preserve the last complete report.

## Start and supply shapes

```sh
just shacl
just ui
just backend
```

Open [Studio Plugins](http://127.0.0.1:4321/plugins), select the default dataset,
and use the SHACL panel. The bundled `claims-example` job reads these graphs:

- Data: `urn:shacl:example:data`
- Shapes: `urn:shacl:example:shapes`

Upload [shapes.ttl](examples/shapes.ttl) into the Shapes input and
[invalid.ttl](examples/invalid.ttl) into Data. Each input **replaces its selected
graph atomically**, avoiding accumulation of obsolete shapes. Ordinary import
in the SPARQL workspace can also append shapes to a chosen named graph.

Click **Validate · report only**: expect 2 Violations (missing policy, negative
reserve), 1 Warning (missing description), and 1 Info (missing reference).
Click **Run validation gate**: it fails, preserving both the data and the report.
Replace Data with [valid.ttl](examples/valid.ttl), validate again, and the gate
passes with zero findings. Editing the data or shapes makes the report stale.

Shapes are Turtle, for example:

```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <urn:shacl:example:> .
ex:ClaimShape a sh:NodeShape ;
  sh:targetClass ex:Claim ;
  sh:property [ sh:path ex:policy ; sh:minCount 1 ; sh:nodeKind sh:IRI ] ;
  sh:property [ sh:path ex:description ; sh:minCount 1 ;
                sh:severity sh:Warning ; sh:message "Add a description." ] .
```

## Configuration and standalone library/CLI use

The gateway reads `<tenant-config-dir>/<dataset>/shacl.json`. A standalone server
or CLI reads `RDF_SHACL_CONFIG`. Without configuration this plugin is absent.
Restart the server after changing configuration; shape/data edits need no restart.

```json
{
  "endpoint": "http://127.0.0.1:18083",
  "jobs": [{
    "name": "claims-example",
    "data_graphs": ["urn:shacl:example:data"],
    "shapes_graphs": ["urn:shacl:example:shapes"],
    "fail_on": "violation"
  }]
}
```

Use `"default"` for the unnamed graph. Multiple selected graphs are merged for
validation; graph membership is retained in publication metadata but a finding
does not claim which source graph supplied a statement. Data and shapes must
not overlap. Reasoning is not automatic: select an inference output explicitly
if wanted and ensure that inference is current. Empty data or shapes with no
matching target nodes may conform; an empty shapes graph fails explicitly.

```sh
export RDF_SHACL_CONFIG="$PWD/deploy/local/tenants/default/shacl.json"
target/debug/rdftool plugin --source http://127.0.0.1:7878 \
  --plugin shacl --operation validate --input '{"job":"claims-example"}'
# Explicit workflow gate: nonzero exit means stop the next step.
target/debug/rdftool plugin --source http://127.0.0.1:7878 \
  --plugin shacl --operation check --input '{"job":"claims-example"}'
```

HTTP uses existing generic routes:

```sh
curl --fail-with-body -H 'Content-Type: application/json' \
  -d '{"job":"claims-example"}' \
  http://127.0.0.1:7878/api/plugins/shacl/operations/validate
```

Library applications register `Shacl::new(config)` on `rdf_engine::Database` at
composition time, then call `database.invoke("shacl", "validate", input)`.
The plugin depends on the SDK and provider transport, not engine internals.

## Query reports with standard SPARQL

```sparql
PREFIX sh: <http://www.w3.org/ns/shacl#>
SELECT ?focus ?path ?severity ?message WHERE {
  GRAPH <urn:rdftool:derived:shacl:claims-example> {
    ?report a sh:ValidationReport ; sh:result ?finding .
    ?finding sh:focusNode ?focus ; sh:resultSeverity ?severity .
    OPTIONAL { ?finding sh:resultPath ?path }
    OPTIONAL { ?finding sh:resultMessage ?message }
  }
}
ORDER BY ?severity ?focus ?path
```

`status` reports `missing`, `ready`, or `stale`. Both shape and data snapshots are
checked before publication; concurrent source changes fail and require retry.
Reports replace the previous generation and survive reopen and normal RDF
backup. A failed gate saves its complete report before returning an error.
Historical report retention is not implemented.

**Checks do not enforce subsequent writes.** A client can modify data after a
successful check. Native SPARQL updates, uploads, Graph Store, and inference
publication remain independent. Strict atomic rejection needs a generic
pre-commit contract, not an after-write plugin callback.

## Profile and validation

SHACL Core only. SHACL-SPARQL, JavaScript, custom validators, external targets,
imports, and explicit entailment are rejected. No network fetching by shapes.
Property paths use standard RDF path expressions; complex paths may appear as
blank nodes in the table, with their structure available in the RDF report.
This is behavioral integration coverage, not an official conformance claim.
See [ADR 0019](../../docs/adr/0019-shacl-validation.md) for limits and boundaries.

```sh
cargo test --locked -p rdf-plugin-shacl
cargo clippy --locked -p rdf-plugin-shacl --all-targets -- -D warnings
# Real Jena, local temporary database, HTTP, and deployed gateway isolation:
just shacl-test
```
