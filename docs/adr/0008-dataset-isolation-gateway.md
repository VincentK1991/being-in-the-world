# ADR 0008: Dataset-level isolation with one server per dataset behind a gateway

Status: accepted. Chosen over a multi-dataset server (rejected for now) and a full
stack per tenant (kept as an escalation path). The goal is multi-tenancy and
repeatable testing with dataset-level isolation, not named-graph separation, while
keeping the inference and Qdrant semantic-search plugins working for every dataset.

## Context

A dataset is already an isolated database: its own Oxigraph/RocksDB directory named
by a UUID, mapped from a user-facing name by `catalog.json` (see
`rdf-store::engine`). `rdftool serve` opens exactly one dataset, and the engine takes a
RocksDB lock, so one process owns a dataset at a time. The plugins are already
dataset-aware:

- Inference writes deductions to `urn:rdftool:derived:inference:<job>` inside the
  dataset it runs on; the OWL-RL and Jena sidecars receive triples per request and
  keep no per-dataset state.
- Semantic search names Qdrant aliases and collections
  `rdfvs_<sha256(dataset identity)[..32]>_<index>`, so two datasets cannot collide in
  one Qdrant.

What was missing: a way to run and reach many datasets, per-dataset plugin settings
(configuration is read once per process), dataset list/delete, and cleanup of Qdrant
collections when a dataset is removed. The architecture notes (ADR 0002) record that
plugin lifecycle hooks do not exist, so removal cannot rely on one.

## Decision

Run **one ordinary `rdftool serve` process per dataset** and put a small gateway in
front of them. Shared services stay shared: one Qdrant, one embedding service, one
OWL-RL sidecar and one Jena sidecar serve all datasets.

```text
Studio / clients
      |
   nginx :4321 ---- /d/<dataset>/*, /api/datasets, other paths
      |
 rdftool gateway :7878/7879          (catalog, routing, lifecycle)
      |  spawns on first use, one loopback port each
      +-- rdftool serve --dataset alpha  -- own RocksDB, own plugin config
      +-- rdftool serve --dataset beta   -- own RocksDB, own plugin config
      +-- rdftool serve --dataset default
                |                 \
        shared Qdrant / embeddings   shared OWL-RL / Jena sidecars
```

### Gateway (`rdftool gateway --db ROOT`)

- `GET /api/datasets` lists catalogued datasets: name, id, whether the server is
  running, and quads and named-graph counts for running ones. It never opens a
  stopped dataset.
- `POST /api/datasets` `{"name": ...}` creates a dataset (201, 409 if it exists, 400
  for an invalid name). Names are 1-64 of letters, digits, `-`, `_`, `.`, not starting
  with `.`, because they appear in URLs, config paths and plugin namespaces.
- `DELETE /api/datasets/<name>` stops that dataset's server, removes the catalog
  entry and directory, then runs plugin cleanup (below). The default dataset cannot
  be deleted. Deletion refuses while any process still holds the dataset's lock.
- `/d/<name>/<path>` is forwarded to that dataset's server (404 for unknown names;
  the gateway never creates a dataset on a read). Any other path goes to the
  default dataset, so existing clients, tests and scripts need no change.
- `/health` is answered by the gateway itself, independent of any dataset.
- Children are started with the same executable, bound to a free loopback port, and
  stopped with SIGTERM (then SIGKILL after 15 s) on delete and on gateway shutdown.
  The gateway binds loopback only, like the servers.

### Plugins per dataset

- Global `--semantic-config` and `--inference-config` apply to every dataset.
- With `--tenant-config-dir DIR`, `DIR/<dataset>/semantic.json` and
  `DIR/<dataset>/inference.json` replace the global file for that dataset when
  present. Different tenants can therefore have different inference jobs or vector
  indexes without touching the engine. Configuration is still read at server start,
  so a change takes effect when that dataset's server restarts.

### Dataset lifecycle

- CLI: `rdftool datasets --db ROOT list | create NAME | delete NAME --yes` operates on
  the catalog directly. Deleting an in-use dataset fails.
- Plugin cleanup is explicit and outside the engine: semantic-search exposes
  `purge_dataset(qdrant_url, identity)`, which deletes every Qdrant collection whose
  name carries the dataset's prefix. The CLI and gateway call it on delete (using the
  tenant's own Qdrant URL when overridden). A failure is reported as a warning after the
  data is already gone; re-running cleanup is safe. Inference needs no cleanup because
  its output lives inside the dataset.

### Studio

- A top bar on every page holds the dataset context: it lists, creates, switches and deletes
  datasets and shows quad and graph counts. The graph, SPARQL workspace and Datasets pages are scoped to it. The choice is kept in `localStorage`; all API calls use `/d/<name>/...`, or the
  plain paths when nothing is selected (the default dataset). Switching reloads the page
  so no state leaks between datasets.
- A **Datasets** page lists every dataset with status and counts, and for the
  selected one shows its named graphs, common types and properties, and its plugins
  with operations and properties.
- A plain `rdftool serve` has no dataset API; the switcher then hides itself and
  Studio behaves as before.

## Consequences

- Isolation is physical and per process: separate data, locks, memory, plugin config
  and failure domains. A crashing dataset server does not affect others; the gateway
  restarts it on the next request.
- Cost: one RocksDB instance of memory per *running* dataset. Servers start lazily and
  currently stay up until the gateway stops; an idle-shutdown timeout is a natural
  next step (see below). Each request through the gateway buffers its body (bounded by
  `--max-body-bytes`), which is fine for the local-first scope.
- The default dataset exists from gateway start and keeps the unprefixed API working.
- Nothing in the engine, plugin SDK or `serve` changed; the gateway only composes them.

## Testing and operations

- `Dataset::open` per test root gives isolation without any server; the gateway test
  (`crates/rdf-cli/tests/gateway.rs`) covers create, isolation, listing, 404 and 409
  cases, and deletion. Unique dataset identities mean parallel tests cannot collide in
  a shared Qdrant; run `datasets delete` (or `purge_dataset`) afterwards to reclaim vectors.
- Docker: the backend container runs `gateway` instead of `serve`; nginx and the Vite dev
  server proxy `/d/` to it. Existing volumes keep working because the catalog format is
  unchanged.

## Alternatives considered

- **One server hosting many datasets.** Best density for hundreds of tenants, but it
  needs a multi-database server, per-request plugin construction and the plugin
  lifecycle hooks ADR 0002 defers. Revisit if tenants outgrow one process each; the
  plugins already take a dataset identity, so they carry over.
- **A full stack per tenant** (own Qdrant and sidecars). Strongest isolation and the
  simplest deletion, at much higher operating cost. Use it for a tenant with hard
  isolation or noisy-neighbour requirements; the gateway's per-tenant config already
  allows pointing one tenant at a separate Qdrant.

## Follow-ups

- Idle shutdown of dataset servers after a configurable quiet period.
- Streaming proxy bodies and a per-tenant concurrency budget.
- Authentication and per-tenant quotas if the service ever leaves loopback.
