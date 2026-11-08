# Inference providers: SPARQL and Jena rules

These adapters are registered by application composition under the existing
`inference` plugin. The database engine and HTTP backend contain no rule-specific
code. Jobs share snapshot isolation, source/config fingerprints, atomic publication,
stale detection and explicit rebuilds.

| kind | profile | Execution |
| --- | --- | --- |
| `owlrl` | `rdfs-v1`, `owlrl-core-v1` | Existing OWL-RL sidecar |
| `jena-rules` | `jena-forward-v1` | Stateless Jena 6.2.0 / Java 21 sidecar |
| `sparql-rules` | `sparql-construct-once-v1` | Isolated in-memory Oxigraph 0.5.13 in this process |

## Configuration

See [the complete example](../inference/example-rules-config.json). Each job names
a provider, selects source graphs, and includes rule documents as JSON strings.
Rule text is part of the compatibility fingerprint; editing it marks existing
output stale after restarting/reloading the application with the new config.
Configuration is loaded at startup, not watched for changes.

```json
{
  "providers": {
    "local": {"kind": "sparql-rules", "config": {"cancel_after_seconds": 30, "max_triples": 200000}},
    "java": {"kind": "jena-rules", "config": {"endpoint": "http://127.0.0.1:18083"}}
  },
  "jobs": [{
    "name": "work",
    "provider": "local",
    "profile": "sparql-construct-once-v1",
    "source_graphs": ["urn:rules:data"],
    "rules": ["CONSTRUCT { ?p <https://example.org/worksInCountry> ?c } WHERE { ?p <https://example.org/worksFor> ?o . ?o <https://example.org/locatedIn> ?c }"]
  }]
}
```

Use `--inference-config PATH` or `RDF_INFERENCE_CONFIG`. Legacy top-level
`endpoint` configurations remain accepted as OWL-RL jobs, but cannot be mixed with
`providers` or per-job provider names. Provider-aware fingerprinting requires a
one-time rebuild of older inference outputs; assertions are unchanged.

Rules are inline in JSON in this release. The `.rq` and `.rules` files in the
examples are readable copies, not automatically loaded by the configuration.
Each job allows at most 64 rule documents / 1 MiB of rule text.

## Run

```sh
cargo build --locked -p rdf-cli
# Only required for Jena jobs. No Fuseki or Jena database is deployed.
docker compose -f sidecars/jena-rules/compose.yml up --build -d
curl --fail http://127.0.0.1:18083/health
python3 plugins/inference/examples/rules/run.py --output target/my-rule-demo
# SPARQL jobs also work without any sidecars:
python3 plugins/inference/examples/rules/run.py --sparql-only --output target/my-sparql-demo
```

The runner refuses an existing database. It retains the database and exact query
outputs in its output directory. [Recorded examples](../inference/examples/rules/README.md).

```sh
export RDF_INFERENCE_CONFIG="$PWD/plugins/inference/example-rules-config.json"
target/debug/rdftool plugin --source target/my-rule-demo/db --plugin inference \
  --operation rebuild --input '{"job":"work-sparql"}'
target/debug/rdftool plugin --source target/my-rule-demo/db --plugin inference \
  --operation status --input '{"job":"work-sparql"}'
target/debug/rdftool serve --db target/my-rule-demo/db --bind 127.0.0.1:7878
```

While serving, use `--source http://127.0.0.1:7878` for rebuild/status operations.
The generic HTTP endpoint is `/api/plugins/inference/operations/rebuild` with
`{"job":"work-sparql"}`. Studio can upload source data and query the inferred
named graphs; rebuild management remains CLI/HTTP, not a dedicated UI panel.

## SPARQL semantics

Every configured rule must be `CONSTRUCT`. All rules read the **same original
snapshot**, a merged default graph of selected sources. They do not see another
rule's output, regardless of array order. There is no fixed-point loop. Output is
the union of constructed triples minus triples already asserted in the snapshot.
Duplicates across rules are removed before publication.

Joins, filters, arithmetic, OPTIONAL, aggregates and SPARQL negation retain engine
semantics. `FILTER NOT EXISTS` operates over the selected finite snapshot; it is
not OWL open-world inference. `GRAPH` sees no named source graphs in this merged
snapshot. FROM/FROM NAMED, SERVICE (including SILENT and nested forms), updates,
SELECT, ASK and DESCRIBE rule documents are rejected. No semantic/custom-property
plugins are registered inside the isolated evaluation store.

Blank nodes from the source retain their identifiers. Fresh template blank nodes
and nondeterministic SPARQL functions can produce different output on rebuild;
use stable IRIs/deterministic rules when repeatability matters.

The provider uses a private in-memory adapter instance; it never opens live
application database files. It limits source input to 100,000 triples / 16 MiB and
output to `max_triples` (1–200,000) / 16 MiB. **Intermediate query memory is not
hard bounded.** `cancel_after_seconds` (1–60, default 30) requests cancellation from
Oxigraph and checks it when collecting results/before return. **This is cooperative,
not a hard wall-clock timeout:** eager joins and aggregates can delay observing it.
A stress test demonstrated this limitation. An overdue result is rejected once
control returns; the host does not publish partial output. Hard CPU/memory isolation
for arbitrary SPARQL requires a future subprocess execution mode.

## Jena rule semantics

The sidecar wraps `GenericRuleReasoner` in `FORWARD_RETE` mode. It applies rules
recursively to a fixed point within its supported positive finite-domain profile.
Unlike SPARQL one-pass mode, rule B may use a fact derived by rule A.

```text
[worksIn:
  (?person <https://example.org/worksFor> ?company)
  (?company <https://example.org/locatedIn> ?country)
  -> (?person <https://example.org/worksInCountry> ?country)
]
```

Supported rules have nonempty triple-pattern bodies and heads; every head variable
must occur in a body triple. URI/literal constants and variables are supported.
Built-ins, procedural actions, functors, nested/backward rules, wildcard/blank-node
constants, unbound head variables and `@include` imports are rejected. At most 256
parsed rules are accepted. This is a custom rule profile, not automatic OWL
reasoning or full Jena rule-language exposure.

Jena retains source blank-node labels and literal lexical forms. Only deductions
not already asserted are returned. No rule can mutate the live RDF database;
publishing uses the same host transaction as other inference providers.

The container is stateless and has no database volume. It accepts one request at
a time, caps JSON bodies at 32 MiB, input at 100,000 triples / 16 MiB and output at
200,000 triples / 16 MiB. Each job runs in a Java subprocess with `-Xmx384m`, killed
after 60 seconds; the container has 640 MiB memory and two CPUs. Closure can exhaust
resources before output checks: failure preserves the previous inferred graph.

## Extension and validation

`ReasonerRegistry` and `ReasonerFactory` live in the inference plugin's public
interface. The application registers factories, and the plugin routes jobs by
provider name. New provider crates require only application registration, without
editing inference orchestration, database engine or HTTP handlers. HTTP providers
can reuse `HttpReasonerFactory`; independent providers implement `Reasoner` and
`infer_rules`. Provider identity advertises supported profiles.

```sh
cargo test -p rdf-inference-providers
cargo test -p rdf-inference-providers --test jena_live -- --ignored --nocapture
python3 sidecars/jena-rules/test_service.py -v
npm test --prefix scripts/ui -- --rules
```

The live tests need the Jena sidecar; the browser test needs built Studio assets,
`rdftool` and Playwright Chromium. The native SPARQL tests need no external service.
See [architecture](../../docs/adr/0007-inference-rule-providers.md) and
[validation](../../docs/runbook/2026-11-02-rule-providers-validation.md).
