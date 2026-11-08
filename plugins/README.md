# Developing database plugins

[Semantic search](semantic-search/README.md) demonstrates a stateful plugin whose
derived vector index lives in a Qdrant sidecar, with local CPU/CUDA embeddings.
It uses the same property SDK; explicit management operations rebuild its owned
index while the RDF context stays read-only.

**SDK v2 adds SPARQL property functions.** See the working
[custom-properties plugin and developer contract](custom-properties/README.md).
The read-only JSON-operation instructions below continue to apply in v2; manifests
also include `properties`, which the host populates from registered implementations.

Read [the architecture contract](../docs/adr/0002-engine-plugins-and-studio.md)
first. `dataset-summary` is the working SDK v1 example. It imports only
`rdf-plugin-api`, `anyhow` and `serde_json`; it has no dependency on HTTP, Oxigraph,
or engine internals.

To add a v1 read operation:

1. Create a crate under `plugins/` and add it to the Cargo workspace.
2. Implement `Plugin::manifest` with a unique lowercase/hyphen ID, version,
   `SDK_VERSION`, and unique operation names.
3. Implement `Plugin::invoke` using the supplied `ReadContext`; validate JSON
   inputs and return structured errors. Context belongs to the active dataset.
4. Register your implementation in the application composition root before
   sharing the database (`rdf-cli`'s `Serve` branch demonstrates registration).
5. Test it directly against an in-memory `rdf_engine::Database`, including empty
   data, two isolated datasets, invalid input and unsupported operations.

The engine and server require no edits for a new implementation of this contract.
Registration is compile-time application configuration, not runtime installation.

Example requests against the default application:

```sh
curl http://127.0.0.1:7878/api/plugins
curl -H 'Content-Type: application/json' -d '{}' \
  http://127.0.0.1:7878/api/plugins/dataset-summary/operations/count
```

Unknown plugins/operations return 404, malformed or rejected inputs return 400,
unsupported media types return 415, body limits return 413, and exhausted engine
capacity returns 503. The sample count operation returns SPARQL Results JSON.

Stateful reasoning, write hooks, indexes, scalar SPARQL functions and lifecycle
callbacks are not implemented SDK v2 features. Define their semantics and extend
the SDK deliberately before developing plugins that need them.

## Discovery

Every dataset reports its plugins to queries and to federation engines (ADR 0013):

```sparql
SELECT ?plugin ?kind ?name WHERE { (?plugin ?kind ?name) <urn:rdftool:plugins:provides> () }
```

A plugin that configures named things (indexes, jobs, ...) implements `Plugin::facets` so they appear
in that list and in the Service Description at `GET /query`. The catalog plugin is registered last.
