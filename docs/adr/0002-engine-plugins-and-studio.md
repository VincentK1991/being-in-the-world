# ADR 0002: Independent database runtime, plugins, HTTP backend, and Studio

Status: accepted. This supplements ADR 0001; Oxigraph remains the storage/query engine.

Update: [ADR 0003](0003-custom-property-functions.md) extends the SDK to v2 with
bounded SPARQL property functions. The v1 section below records the original scope;
the v2 property contract supersedes its deferral of that extension point.

## Rule

The database is an independent library. The backend exposes it over HTTP. Studio
is a client of that backend. Plugins extend the database through an SDK.

**Developing a plugin must not require editing engine implementation, HTTP
handlers, or existing plugins. A new extension point requires a deliberate SDK
and engine API change.** Application composition registers selected plugins.

```text
Astro / React / TypeScript / Tailwind Studio
                    | HTTP
               rdf-server (Axum)
                    | public library API
                 rdf-engine <---- embedded Rust applications
                  /       \
        rdf-store          rdf-plugin-api
     Oxigraph adapter           |
            |              plugins/*
     Oxigraph / RocksDB
```

## Ownership and dependency rules

| Component | Owns | Forbidden dependencies / responsibilities |
| --- | --- | --- |
| `apps/studio` | UI, browser state, typed HTTP client | Filesystem database access, engine execution |
| `rdf-server` | HTTP parsing/serialization, body limits, admission control, static assets | Direct `rdf-store`/Oxigraph access, plugin-specific routes, RDF transactions |
| `rdf-engine` | Public database API, dataset-scoped execution, plugin registration/dispatch | Axum, React, individual plugin implementations |
| `rdf-store` | Existing Oxigraph adapter, catalog, persistence, migration and backup | HTTP and UI |
| `rdf-plugin-api` | Versioned public plugin contracts | Axum, Oxigraph, RocksDB, engine internals |
| `plugins/*` | Each extension's behavior | Engine/server source changes, database file access |
| `rdf-cli` | Application composition and command-line interface | HTTP-specific database semantics |

`rdf-store` retains its current name and migration code to avoid an unrelated
compatibility break. It fills the proposed `rdf-engine-oxigraph` role; extraction
into that name is a future mechanical cleanup. Existing maintenance/legacy CLI
commands still use this adapter. Server execution now exclusively uses
`rdf-engine`. Some engine result/value types remain adapter re-exports; the plugin
SDK intentionally exposes none of those types.

Process separation is optional. In this release the Axum server embeds the
database library. The engine can execute with no HTTP runtime. Production Studio
is static Astro output with a hydrated React workbench, embedded into the Rust
binary at build time. Development uses Astro's local server with an API proxy.

## Plugin SDK v1: implemented scope

- Trusted Rust plugins registered at application startup; installation requires
  rebuilding the application. No dynamic Rust ABI, WASM loading, or hot reload.
- Manifest: ID, implementation version, SDK version, declared operation names.
  Registration rejects incompatible SDK versions, invalid/duplicate IDs and
  invalid/duplicate operations. Metadata is captured once at registration.
- Dataset-scoped read context supports SELECT/ASK JSON results, with the same
  SERVICE/LOAD policy as ordinary execution. Each query has its own snapshot;
  multiple context calls do not form a shared transaction.
- JSON operations are invoked through the public library API or generic HTTP
  routes. The same HTTP body/concurrency limits apply to plugin calls.
- `dataset-summary` is an independent example package depending only on the SDK.
  Its `count` operation accepts `{}` and counts default and named-graph quads.
- Plugin discovery appears in Studio's sidebar. Individual plugin panels are not
  part of this release. Plugins can be used without Studio or Axum.

Native plugins execute as trusted application code. The read context limits the
SDK surface, **not OS privileges**. There is no sandbox, enforced per-plugin CPU
or memory quota, timeout cancellation, or guaranteed containment of plugin panics.
Implementations must be thread-safe and validate their operation inputs.

## Required designs before expanding the SDK

The following are contracts to design and test before implementing the relevant
plugin types, not existing capabilities:

1. Typed importer, pre-commit validator, query-function, reasoning and derived-index
   extension points. Arbitrary SPARQL syntax/planner changes are not promised.
2. Transactional validation runs before commit; failure aborts the write. No
   callback may recursively mutate the transaction. After-commit processing needs
   a durable change log, idempotent replay and explicit consistency watermarks.
3. Dataset-scoped plugin state, ownership, schema migrations, backup/restore
   participation, index rebuilds and interrupted-write recovery.
4. Lifecycle hooks for initialization, dataset open/close, recovery and shutdown;
   dependency resolution, configuration schemas and compatibility policy.
5. Capability declarations, observability and resource controls. Untrusted or
   runtime-installed plugins require a separately designed WASM/process boundary.
6. A shared `rdf-plugin-testkit` for the above contracts, including dependency
   failures, dataset isolation, rollback, restart, migration and backup recovery.
   V1 currently has focused engine and live HTTP contract tests.

The backend must remain generic as these capabilities grow. New plugin-specific
Axum routes are not the integration mechanism. Custom plugin UI contributions
require a separate frontend contract and cannot execute native plugin code.

## Validation

`rdf-engine` tests execute a plugin without HTTP and verify dataset isolation,
duplicate registration, invalid operations/inputs and query restrictions. The
HTTP test covers discovery, invocation and malformed/unsupported requests. Studio
browser tests use a real persistent database. These checks establish boundaries
and behavior; they do not imply full RDF/SPARQL standards conformance.
