# ADR 0013: Plugin discovery from SPARQL and the service description

Status: accepted.

## Context

Federated queries call other datasets, and each dataset can have different plugins, search
indexes and inference jobs (ADR 0008). Until now the list was only available over HTTP
(`/api/plugins`), so a query could not tell whether a dataset offered `search:hybrid` on an
index called `narratives` before calling it.

## Decision

One catalog, two surfaces.

- **Catalog.** Each plugin may declare *facets* (`Plugin::facets`: index names, job names,
  virtual-graph names). `Database::catalog()` flattens manifests, property functions and facets
  into rows `(plugin, version, kind, name)`. Kinds: `plugin`, `operation`, `property-function`,
  `index`, `job`, `virtual-graph`. This is an additive SDK change; the SDK version stays 3.
- **Property functions** from the new `plugin-catalog` plugin, registered last so it sees the rest:
  - `(?plugin ?version) <urn:rdftool:plugins:available> ()`
  - `(?plugin ?kind ?name) <urn:rdftool:plugins:provides> ()`

  Because a property function runs on whichever endpoint evaluates the query, `SERVICE` asks the
  remote dataset about itself. A dataset without the plugin returns no rows, not an error, so a
  query can be written as "only if it has it" by joining on the catalog row.
- **Service Description.** `GET /query` with no `query=` returns a SPARQL 1.1 Service Description
  generated from the same catalog, with plugins in the `urn:rdftool:vocab:` vocabulary
  (`rdftool:plugin`, `rdftool:id`, `rdftool:version`, `rdftool:operation`,
  `rdftool:propertyFunction`, `rdftool:index`, `rdftool:job`, `rdftool:virtualGraph`).
  Federation engines and generic tools read it without custom code.

## Consequences

- The catalog is a snapshot taken at startup, like plugin configuration. Live state (a stale
  inference job, an index needing a rebuild) still comes from each plugin's `status` operation.
- The vocabulary is a public contract; add terms, do not rename them.
- Endpoint URLs of the federation and virtual-graph plugins are deliberately not published.

## Alternatives

A virtual named graph (`urn:rdftool:plugins` graph) was rejected: it needs an engine hook, and
Comunica reads named graphs poorly (see ADR 0010). A gateway-wide catalog across datasets can be
added later as one more allowlisted endpoint.
