# Local Docker deployment

This deployment targets this Ubuntu 22.04 WSL environment with Docker Desktop
WSL integration. From `being-in-the-world/repo`:

```sh
just                 # list commands; does not run CI
just inference       # optional Jena and OWL-RL sidecars
just semantic-gpu    # optional Qdrant + CUDA model; use semantic-cpu without GPU
just backend         # build Rust, deploy backend + persistent Oxigraph store
just ui              # build/check Astro in Docker, deploy Studio
just demo            # add examples and rebuild/validate configured plugins
just status
```

Open **http://127.0.0.1:4321/**. The backend is at
**http://127.0.0.1:7878/** (`/health`, `/query`, `/update`, `/store`, `/api/*`).
The UI supports RDF upload and SPARQL queries. Choose the inference and semantic
query templates after loading the demo. `just demo` adds known sample triples;
it never clears existing user graphs. It rebuilds the configured demo indexes
and inference jobs. Saved query results: `target/local-deployment/`.

## Components and persistence

- `rdf-local-ui-1`: Nginx serves the built Astro/React/TypeScript/Tailwind app and
  proxies same-origin requests to the backend. No CORS configuration is needed.
- `rdf-local-backend-1`: Axum uses the independent `rdf-engine` library, its
  plugin API and the Oxigraph adapter. The database engine is a library inside
  this process, not a second network database daemon. UI code is independent.
- Volume `rdf-local_rdf-data`: persistent catalog and default dataset at
  `/data/db`. Recreating containers preserves it. Existing filesystem demo
  databases are not moved or overwritten.
- Existing separate Docker projects provide Jena, OWL-RL, Qdrant and embeddings.
  Their volumes and lifecycle are independent of `just down`.

`just logs` follows UI/backend logs. `just down` removes those containers and
preserves database volumes. Do not use `down -v` unless intentionally deleting
the data. The running backend owns the store lock; use HTTP clients while it runs.

The backend uses the checked-in example inference and semantic configurations
copied into the backend image. Edit them and rerun `just backend` to apply changes.
This avoids stale WSL bind mounts after Docker Desktop restarts. Local
CUDA embeddings are selected; no text is sent to a hosted embedding provider.
Missing optional sidecars cause their plugin operations to fail explicitly;
ordinary imports and SPARQL remain available.

## Build and networking details

`just backend` compiles a locked development binary on the Linux host, then
packages it in an Ubuntu 22.04 runtime image. Install Rust, Clang/libclang and
CMake using `scripts/setup-linux.sh`. This is a local development image, not a
portable release build: a host with a newer incompatible glibc or different
architecture needs a matching build/runtime environment. `just ui` builds with
Node 22.23.4 inside Docker; it does not use an outdated host Node installation.
Docker and Compose v2 with `--wait` are required.

Docker publishes only host `127.0.0.1:4321` and `127.0.0.1:7878`. Within the
backend container, Axum retains its loopback bind on port 7879. A transport relay
on container port 7878 forwards incoming requests to it. Loopback relays for
configured providers forward to Docker Desktop's `host.docker.internal`, where
existing sidecars have their localhost ports published. `tini` and the entrypoint
forward shutdown signals and terminate the relay processes with the backend.
The core engine's non-loopback bind and outbound-query restrictions are unchanged.

Docker Desktop's host networking was tested and was not reachable from this WSL
terminal; explicit published ports are required here. This Compose configuration
uses Docker Desktop's host gateway behavior.

On native Linux Docker, `host-gateway` alone is not enough, because loopback-published ports are
not reachable through the bridge gateway. Every Compose file publishes on
`${RDF_PUBLISH_HOST:-127.0.0.1}`, and the backend maps `host.docker.internal` to `host-gateway`.
The nightly CI job sets `RDF_PUBLISH_HOST=0.0.0.0` on its throwaway runner (see
`2026-11-08-live-stack-ci.md`). Locally the default stays loopback. From WSL, environment
variables reach Docker Desktop's CLI only through `WSLENV`.

The Nginx upstream uses Docker DNS with periodic resolution so backend recreation
does not require rebuilding the UI. A 300-second proxy read timeout is a transport
limit, not a promise that a timed-out engine computation is cancelled.

## Provenance status

The [confirmed intent](../intent/inference-provenance.md) and
[implementation specification](../specs/inference-provenance.md) are saved.
Fact-level rule evidence is still planned. The deployed inference plugin provides
deductions and generation metadata; it does not yet expose per-fact proofs.

If the Linux Docker socket is unavailable while Docker Desktop is running, the
recipes automatically use its Windows CLI with converted Compose paths. No
Docker daemon restart or changes to unrelated containers are needed.

## Validation recorded 2026-11-07

- `just backend` and `just ui` succeeded using the Windows CLI fallback after
  Docker Desktop's WSL socket became unavailable. Both containers are healthy.
- Configuration file bind mounts had become stale after the environment restart;
  copying configuration into the image fixed startup while preserving the data
  volume. Only the RDF backend/UI were recreated.
- The UI image's Astro checks and build passed.
- `scripts/ui/deployed.mjs` used a real Chromium browser against port 4321:
  connected, discovered plugins, uploaded Turtle into a temporary named graph,
  queried the uploaded literal, and displayed Alice/Europe and Bob/NorthAmerica
  from Jena deductions. No browser page errors. Temporary graph removed afterward.
- A complete ordered RDF query returned the same 181 quads before and after a
  backend restart. Inference and semantic index status remained `ready`; a real
  local-model semantic query returned the electric car as its highest ranked hit.
- Earlier `just demo` execution matched four saved inference result fixtures and
  executed seven semantic query patterns using CUDA embeddings and Qdrant.
- These are deployment/behavior checks, not a fresh full-workspace conformance run.

Screenshot: `target/local-deployment/studio.png`. Re-run the browser check with
Node 22 and Playwright installed (`npm ci --prefix scripts/ui`, then install its
Chromium browser if needed):

```sh
node scripts/ui/deployed.mjs
```

`PLAYWRIGHT_MODULE` can point to an existing Playwright installation. The test
expects the examples from `just demo`; it creates and removes only its own unique
temporary graph. It leaves the running services available for interactive use.
