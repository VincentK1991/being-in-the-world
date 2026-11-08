# Graph-scoped inference

The inference plugin materializes deductions with a pinned RDFLib OWL-RL Docker
sidecar. RDF and inferred graphs remain in Oxigraph. The backend uses existing
generic plugin routes; there are no inference-specific HTTP handlers.

## Start and use

From the repository root:

```sh
docker compose -f sidecars/inference/compose.yml up --build -d
curl --fail http://127.0.0.1:18082/health
cargo build --locked -p rdf-cli

# The runner refuses an existing database; choose a fresh directory per run.
python3 plugins/inference/examples/run.py --output target/my-inference-demo
```

To use your own data:

```sh
target/debug/rdftool ingest --from plugins/inference/example.trig --to ./inference-db
export RDF_INFERENCE_CONFIG="$PWD/plugins/inference/example-config.json"
target/debug/rdftool plugin --source ./inference-db --plugin inference \
  --operation rebuild --input '{"job":"vehicles"}'
target/debug/rdftool plugin --source ./inference-db --plugin inference \
  --operation status --input '{"job":"vehicles"}'
```

`--inference-config PATH` is the equivalent global CLI option. Configuration is
loaded at application startup. The CLI requires persistent sources for inference.
Library applications can also register the plugin against an in-memory database.

```json
{
  "endpoint": "http://127.0.0.1:18082",
  "jobs": [{
    "name": "vehicles",
    "profile": "owlrl-core-v1",
    "source_graphs": ["urn:example:data", "urn:example:ontology"]
  }]
}
```

Each job explicitly merges its selected asserted/ontology graphs for reasoning.
Use `"default"` to select the default graph. Unselected graphs do not participate.
A source may be another job's output graph (`urn:rdftool:derived:inference:<job>`), which chains the jobs;
see [Chained jobs](#chained-jobs). No other part of the `urn:rdftool:derived:` namespace can be a source.
Remote ontology imports are deferred. Blank-node graph names cannot
be selected in this release; blank-node subjects/objects retain their identities.

## Query and Studio

```sparql
PREFIX ex: <https://example.org/>
SELECT ?vehicle WHERE {
  GRAPH <urn:rdftool:derived:inference:vehicles> {
    ?vehicle a ex:Vehicle .
  }
}
```

The example returns `ex:car1`, inferred from its asserted `ex:ElectricCar` type.
See [executed examples](examples/README.md) for inheritance, inverse relationships,
transitivity and graph isolation.

Start Studio with the same config environment or CLI option:

```sh
target/debug/rdftool serve --db ./inference-db --bind 127.0.0.1:7878
# While serving, use HTTP instead of opening the database from another process:
target/debug/rdftool plugin --source http://127.0.0.1:7878 --plugin inference \
  --operation rebuild --input '{"job":"vehicles"}'
```

Studio discovers the plugin automatically. Upload data and run the SPARQL query
above; rebuild through CLI/HTTP. There is no dedicated inference-management panel.
The equivalent endpoint is `POST /api/plugins/inference/operations/rebuild` with
`{"job":"vehicles"}`. `status` uses the same generic endpoint structure.

## Declared profiles

- `rdfs-v1`: RDFLib's finite RDFS closure, without additional axiomatic/datatype
  axiom expansion. OWL vocabulary is rejected in this profile.
- `owlrl-core-v1`: a deliberately bounded OWL-RL rule profile for named class and
  property hierarchies, domain/range, equivalent classes/properties, inverse,
  symmetric and transitive properties, equality, functional/inverse-functional
  properties, and the reasoner's supported disjointness/difference checks.

The exact allowed OWL terms are: `Class`, `Ontology`, `NamedIndividual`,
`ObjectProperty`, `DatatypeProperty`, `Thing`, `Nothing`, `inverseOf`,
`TransitiveProperty`, `SymmetricProperty`, `equivalentClass`, `equivalentProperty`,
`sameAs`, `FunctionalProperty`, `InverseFunctionalProperty`, `disjointWith`,
`differentFrom`, and `propertyDisjointWith`. Class/property expressions in
subclass/subproperty/equivalence statements must be named IRIs. Other OWL terms
are rejected, including imports, restrictions, union/intersection expressions,
cardinalities and property chains. This conservative check also rejects an
unsupported OWL term merely mentioned as a URI in the selected data.

`owlrl-core-v1` is **not full OWL 2 RL**. Its guard is a bounded vocabulary and structure
check, not a complete OWL structural validator.

Two later profiles drop the guards. A job opts in by naming one; `rdfs-v1` and `owlrl-core-v1`
never change, because the profile and the reasoner's identity are part of every job's fingerprint.

- `rdfs-v2`: RDFS, with OWL terms treated as plain IRIs instead of rejected.
- `owlrl-full-v1`: all of OWL 2 RL as the owlrl library implements it. This includes intersections,
  unions, restrictions (`someValuesFrom`, `allValuesFrom`, `hasValue`, max cardinality), property
  chains, keys, `AllDisjoint*`, `AllDifferent`, negative property assertions and annotations.

Both add axiomatic triples, scoped:

- Nothing is said about an RDF, RDFS, OWL or XSD term that the input never mentions.
- Nothing is said that holds of every resource (`x a rdfs:Resource`, `x a owl:Thing`).
- Reflexive `owl:sameAs` stays, because queries that walk `sameAs` chains rely on it.

On the ACME data, `owlrl-full-v1` derives 393 triples against `owlrl-core-v1`'s 409. It drops the
core profile's vocabulary chatter and adds class and domain/range conclusions.

Conformance (W3C OWL 2 RL tests, RDF 1.1 Semantics, SPARQL entailment regimes; see
`docs/conformance.md`):

| Profile | Pass | Fail | Not run |
|---|---|---|---|
| `owlrl-core-v1` | 59 | 20 | 165 |
| `owlrl-full-v1` / `rdfs-v2` | 155 | 28 | 61 |
| native `owl2rl-v1` (below; no RDFS profile) | 134 | 20 | 90 |

The remaining failures are mostly beyond rule-based OWL 2 RL, for example `differentFrom` by
contradiction or conclusions that a class expression exists. `docs/owl-coverage.md` lists each one.

### Inconsistent input

An inconsistency is a result, not a failure (sidecar protocol 2). `rebuild` returns
`"state": "inconsistent"` with `clashes`, each a `message` and the `terms` it names. The derived
graph is left exactly as it was.

`status` keeps reporting `inconsistent`, with the clashes and `inconsistent_since`, while the job's
sources, inputs and configuration are the ones that clashed. Change any of them and the job reads
as stale again; a successful rebuild clears the record. The record is kept in memory, so a restart
forgets it and the job reads as stale.

A cascade never runs a job that reads an inconsistent one. Its step reads `blocked`, with
`blocked_by`.

OWL is open-world: no clash is not proof of consistency, and missing facts are not validation
failures. For closed-world checks use the SHACL plugin.

## Storage, freshness and recovery

- Deductions only go into `urn:rdftool:derived:inference:<job>`; assertions remain
  unchanged. OWL-RL may also derive vocabulary/self-equality facts, not just the
  business facts in the examples.
- Provenance metadata is stored atomically in the companion graph
  `urn:rdftool:derived:inference:<job>:metadata`. It records selected graphs,
  source content token, config fingerprint, reasoner versions/profile, output
  digest and triple count. This is generation-level provenance, not per-fact proofs.
- Rebuild snapshots selected RDF, reasons outside the write lock, then checks the
  source content token under the host mutation lock and atomically replaces both
  output and metadata. Source changes cause rejection. Failed parsing, reasoning
  or publication preserves the previous graph.
- `status` reports `missing`, `ready` or `stale`, comparing selected source content,
  config/provider identity, and output integrity. It scans selected data and output;
  there is no incremental index or durable change feed. Source tokens are content
  fingerprints, not monotonically increasing transaction IDs.
- Deletions require a full rebuild. **Ordinary SPARQL still reads the old inferred
  graph while stale**; it does not implicitly invoke a reasoner or reject the query.
  Check status or rebuild before using it. The default graph is not automatically
  a union of asserted and inferred graphs.
- Database backups include inferred and metadata graphs because these are RDF.
  After restoring to a different database identity, rebuild to refresh provenance.
  Preserve configuration separately. The sidecar has no persistent database state.
- Native plugins and raw adapter/store APIs are trusted. Source-check/publication
  atomicity covers writes through the supported Dataset/Database APIs. Direct
  mutation through the raw Oxigraph store bypasses the host mutation lock and is
  outside this contract. Ordinary user edits to derived graphs are permitted but
  invalidate their recorded output digest.

The sidecar binds to loopback port 18082, accepts one inference operation at a time,
limits input to 100,000 triples / 16 MiB and derived output to 200,000 triples /
16 MiB. HTTP JSON bodies are capped at 32 MiB. Each job runs in a subprocess killed
on a 60-second timeout; Docker limits the container to 512 MiB and two CPUs. Memory
exhaustion can terminate/restart the service; the host keeps the prior graph.
No automatic retries or background rebuilds are performed.

## Extension point and tests

`Inference::with_reasoner(config, Arc<dyn Reasoner>)` accepts another independently
implemented adapter. `Reasoner` exposes identity/supported profiles and inference;
the plugin and core do not implement reasoning algorithms. Further adapters must
honor the N-Triples/blank-node identity contract. DL-specific classification and
entailment APIs need a separate design, not a claim that finite materialization
provides complete DL query answering.

```sh
cargo test -p rdf-engine
cargo test -p rdf-plugin-inference --test live -- --ignored --nocapture
docker compose -f sidecars/inference/compose.yml exec -T owlrl python -m unittest test_worker -v
npm test --prefix scripts/ui -- --inference
```

The browser test requires a current built binary, Studio assets and Playwright.
See [ADR 0006](../../docs/adr/0006-inference-derived-graphs.md) and
[local validation](../../docs/runbook/2026-11-02-inference-validation.md).

## Native OWL 2 RL provider

The `owlrl-native` provider computes OWL 2 RL in process, with no sidecar. Its profile is
`owl2rl-v1`.

```json
"providers": {"native": {"kind": "owlrl-native", "config": {"cancel_after_seconds": 30, "max_triples": 200000}}},
"jobs": [{"name": "acme-native", "provider": "native", "profile": "owl2rl-v1", "source_graphs": ["..."]}]
```

- **Rules:** the W3C OWL 2 RL/RDF tables for equality, properties, classes, class axioms and
  schema, run to a fixpoint. The datatype rules are not implemented.
- **Output policy:** the same as `owlrl-full-v1`.
- **Agreement with owlrl:** it reaches the same OWL conclusions, apart from owlrl's extra RDFS
  layer (`p a rdf:Property`, `C a rdfs:Class`).
- **Speed:** a 60,000-triple closure takes 0.16 s, against owlrl's 42.8 s.
- **Clashes:** each clash names its `rule` and lists its `premises`. Two different values of a
  functional or max-1 property are a clash when they share a datatype and differ in value.
- **Explanations:** it is the one OWL provider that can explain, described next.

`explain` on a native job returns the rule (`prp-dom`, `cax-sco`, ...), its text and the
premises. A premise the job itself deduced first appears as
`{"triple": ..., "deduced_in_job": {"rule": ..., "premises": [...]}}`, explained in turn down to
triples marked `asserted_in` or `outside_input`. A proof is limited to 12 levels and 200
deduced premises; anything cut off is marked `truncated`. Each triple keeps the first
justification found; others may exist.

## Additional rule providers

The same plugin now supports per-job `sparql-rules` and `jena-rules` providers.
See [setup, execution semantics and limits](../inference-providers/README.md),
[combined configuration](example-rules-config.json), and
[actual results](examples/rules/README.md). SPARQL CONSTRUCT rules run once in an
isolated in-memory store; Jena custom rules use a separate stateless sidecar.
The original OWL-RL profile restrictions remain unchanged. Provider-aware config
fingerprinting requires rebuilding older inference outputs once.

## Planned fact-level provenance

The [confirmed intent](../../docs/intent/inference-provenance.md) and
[implementation specification](../../docs/specs/inference-provenance.md) describe
recording rule evidence as RDF readable through ordinary SPARQL. This is proposed
work; current metadata remains generation-level only.

## Outside inputs

A job can also read RDF from a virtual graph or the federation sidecar by listing `inputs` (a CONSTRUCT query, an
endpoint and a `kind` of `sparql` or `federation`). The fetched triples feed the reasoner but are never stored, and
`status` reports `stale` with `stale_inputs` when the outside data changes. See
[ADR 0012](../../docs/adr/0012-inference-with-outside-inputs.md).

## Chained jobs

List a job's output graph among another job's `source_graphs` and the second job reasons over the first job's
deductions (usually together with the asserted graphs, since a job's output holds only what it added):

```json
{"name": "reserved-claims", "provider": "sparql", "profile": "sparql-construct-once-v1",
 "source_graphs": ["urn:rdftool:examples:acme-insurance", "urn:rdftool:derived:inference:acme"],
 "rules": ["... CONSTRUCT { ?claim rsk:hasReserve ?amount } WHERE { ?claim ins:hasLossReserve ?amount . ?amount a ins:LossReserve }"]}
```

- Dependencies come from `source_graphs`; there is nothing else to declare. Unknown jobs, self-reference and cycles are
  rejected when the configuration loads.
- `status` adds `depends_on`. A job whose upstream job is stale or missing is `stale` with `stale_upstream`, even when
  its own source graphs have not changed yet. A change in an upstream job's output makes the downstream job stale
  through the usual source token.
- `rebuild` accepts `"cascade"`: `upstream` first rebuilds the jobs this one reads, `downstream` afterwards rebuilds the
  jobs that read it, `both` does both. Jobs are visited in dependency order and only those that are no longer ready are
  rebuilt, so a rebuild that changes nothing stops the cascade. The named job is always rebuilt. The response lists the
  `steps` and reports the other jobs in `ran`, so refresh rules written for them fire too. A failing job stops the cascade.
- `lineage` returns what a job reads and what reads it, each with its state, and the order to rebuild in.
- One rebuild runs at a time; a cascade holds that slot for all of its jobs.

See [ADR 0015](../../docs/adr/0015-chained-inference-jobs.md).

## Rule tests

A job may carry `tests`. Each gives data, and what the job must (and must not) produce from it:

```json
{"name": "totals above 10 only",
 "prefixes": "@prefix ex: <https://example.org/> .",
 "given":  "ex:l1 ex:unitPrice 5 ; ex:quantity 3 . ex:l2 ex:unitPrice 2 ; ex:quantity 3 .",
 "expect": "ex:l1 ex:total 15 .",
 "absent": "ex:l2 ex:total 6 .",
 "exact":  true}
```

`given`, `expect` and `absent` are Turtle, sharing `prefixes`. The `test` operation (`{"job": "..."}`, or `{}` for every job
with tests) runs the job's own reasoner and rules over `given` and reports, per test, what was `missing`, what was
`present_but_forbidden` and, with `exact`, what was `unexpected`. It reads nothing from the store and writes nothing, and a
job's tests are not part of its fingerprint, so editing one never makes the job stale. Bad Turtle, blank nodes in
`expect`/`absent` and tests that check nothing are rejected when the configuration loads. A chained job is tested by
putting its upstream conclusions in `given`. Run all of them with `just rule-tests`.

A test with `"inconsistent": true` passes only if the reasoner finds `given` inconsistent. Its result lists the `clashes`.
`expect`, `absent` and `exact` are not checked for such a test.

## Explaining a deduction

`explain` answers "why does this triple hold?":

```json
{"triple": "<https://example.org/p1> <https://risk.example.org/vocab#atRisk> true .", "job": "portfolio-risk", "depth": 3}
```

Only `triple` is required (one N-Triples line, no blank nodes). Without `job` the triple is looked up in every job's
derived graph; `depth` (1 to 5, default 3) limits how far back a trail through chained jobs goes. For a derived triple the
result lists one entry per job that holds it, and per derivation: the `rule` and its text, the variable `bindings`, whether
it was an `aggregate` (then every member counted is listed), and the `premises` it relied on. Each premise says where it
lives: `asserted_in` (a source graph), `outside_input` (a virtual-graph or federation input, which is not stored) or
`deduced_by` (the earlier job that derived it, explained in turn). A triple that is not derived is reported with the graphs
that assert it.

- The explanation is computed by re-running the rules over what the job reads now, so a job that is out of date says
  `stale: true` and its trail may differ from what was published.
- SPARQL rule jobs and native OWL 2 RL jobs name the rule and premises. Jobs on the OWL-RL and Jena sidecars answer
  `supported: false` with the graphs they read, because those sidecars return conclusions without justifications.
- Premises that depend on property paths, `GRAPH` blocks or blank nodes are marked `incomplete` or left untraced.
- One call makes at most 400 premise lookups (`truncated` is set when it stops early).
