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
Sources may not use the reserved `urn:rdftool:derived:` namespace. Derived-graph
chaining and remote ontology imports are deferred. Blank-node graph names cannot
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

This is **not full OWL 2 RL or OWL 2 DL conformance**. The profile guard is a
bounded vocabulary/structure check, not a complete OWL structural validator.
Reasoner-reported inconsistencies fail publication; absence of reported errors
is not proof of general OWL consistency. OWL uses open-world reasoning; missing
facts are not validation failures. SHACL validation is a separate future plugin.

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
