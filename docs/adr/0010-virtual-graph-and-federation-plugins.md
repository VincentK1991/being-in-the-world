# ADR 0010: Virtual graphs (Ontop) and SERVICE federation (Comunica) as plugins

Status: accepted for the first implementation. Implements the first slice of
`docs/federation-and-virtual-graphs-plan.md`; where that plan's governance and registry work is not yet done it is
listed under "Not done".

## Decision

Two plugins in one crate (`plugins/federation`) control two sidecars. Rust does no SQL work and no federated
optimisation.

- **`virtual-graph`** ‑ an Ontop endpoint over Postgres with an R2RML mapping. Operations: `status` (runs a count
  through the endpoint, reports latency) and `query` (read queries only; `SERVICE` is refused).
- **`federation`** ‑ a Comunica endpoint that evaluates standard `SERVICE`. Operations: `status` and `query`.

Both are registered per dataset, so a dataset only shows (and can use) what it is configured for. The gateway reads
`<tenant dir>/<dataset>/virtual.json` and `federation.json` and starts that dataset's server with
`RDF_VIRTUAL_CONFIG` / `RDF_FEDERATION_CONFIG`.

## Why a separate operation instead of `/query`

The local query path still rejects `SERVICE` (`rdf-store::policy`). Federation is an explicit operation,
`POST /api/plugins/federation/operations/query`, so ordinary queries keep their guarantees and the plugin SDK needed
no change. Routing `SERVICE` queries from `/query` is a later host decision.

## Federation semantics

- Patterns outside `SERVICE` run against the dataset's own endpoint (`local_source`); each `SERVICE` clause runs
  against its endpoint. The plugin returns the SPARQL result plus the list of endpoints used.
- Comunica pushes a whole query to a lone SPARQL source, which would send `SERVICE` to the local endpoint. The sidecar
  adds an always-empty second source so evaluation stays in Comunica.
- Dataset endpoints answer a bare `GET /query` with a SPARQL Service Description. Federation engines request this
  before a `SERVICE` call, so every dataset server is now a valid `SERVICE` target.
- Other datasets are reached as `http://host.docker.internal:7878/d/<dataset>/query`, so each dataset behaves as its own
  business unit; nothing is merged.

- **Known limit, now handled:** Comunica estimates a pattern's size with a default-graph count query that ignores `GRAPH <g>`,
  and returns no rows when that count is zero, so a bare `GRAPH` block inside `SERVICE` used to return nothing silently.
  The sidecar now wraps any `SERVICE` body that contains `GRAPH` in `SELECT * { ... }`, the form Comunica evaluates correctly
  (see ADR 0014). Federated local patterns outside `SERVICE` still see the default graph only.

## Policy

- The `SERVICE` target IRI is the actual endpoint. An exact allowlist is checked in the plugin before any request
  (walking the query algebra, including `OPTIONAL`, `UNION`, sub-selects and `FILTER EXISTS`) and again in the sidecar
  (which parses the query itself). `SERVICE ?var`, updates, `FROM`/`FROM NAMED`, redirects and oversized bodies are refused.
- Sidecar URLs must be loopback; the backend reaches them through the same relays as Qdrant (ports 18090, 18095).
- Ontop connects with a read-only Postgres role. Federation never writes remote data.

## Verification

`scripts/test/federation_e2e.py` (`just federation-test`) runs against the live stack:

- virtual-graph results equal the same data materialised in the store from the same R2RML mapping;
- a three-source join: `claims` dataset + Postgres through Ontop + `finance` dataset, and the reverse direction;
- dataset isolation (the same query from `finance` returns nothing) and per-dataset plugin availability;
- refusals: unregistered endpoint, endpoint inside `FILTER EXISTS`, variable endpoint, update, and a direct call to the sidecar.

Unit tests cover SERVICE-target extraction and config validation.

## Not done

- The RDF registry, approval workflow and `norm:canonicalId` governance from the plan (Phase 0, Phase 5). The allowlist is a file.
- Per-query limits beyond a timeout and body size; delegated user identity; fan-out budgets.
- Studio support for federated queries (the workspace still runs queries through `/query`).
- Postgres is the only SQL dialect exercised; the ACME sample is small (2 policies, 2 claims).
- The Ontop image is Ontop 4.2.2; pinned by digest.
