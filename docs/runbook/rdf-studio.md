# RDF Studio behavior validation

The Astro/React/TypeScript/Tailwind workbench calls the Axum backend, which uses
the public `rdf-engine` API over the persistent Oxigraph adapter. See the
[architecture contract](../adr/0002-engine-plugins-and-studio.md) for plugin boundaries.
Its import endpoint stages parsed quads in an engine transaction, renames incoming
blank nodes to isolate uploads, and commits only after the complete file parses.
Turtle and N-Triples default to the default graph; an optional absolute graph IRI
targets a named graph. TriG/N-Quads retain their graphs and reject a target override.
Repeated ordinary triples are deduplicated by RDF set semantics. Files containing
blank nodes introduce fresh nodes on each import.

## Docker deployment

Use [the local Docker runbook](local-docker.md) for `just backend`, `just ui`,
provider sidecars, persistent volumes and live demo validation.

## Run locally

From the repository root:

```sh
npm ci --prefix apps/studio
npm run check --prefix apps/studio
npm run build --prefix apps/studio
cargo build --locked -p rdf-cli
target/debug/rdftool serve --db ./studio-db --dataset default --bind 127.0.0.1:7878
```

Open http://127.0.0.1:7878/workspace and choose **Load example data**, then **Run query**.
The join returns Alice, Bob, and Carol with their projects. **Import RDF** accepts
a local file; to query a named graph, click its name in the sidebar or use GRAPH.
The UI displays HTTP/parser errors and clears stale query results on failure.
Stop with Ctrl+C before using a direct database CLI command on the same dataset.

## Automated checks

Use Node >=22.23.4 (pinned in `apps/studio/.nvmrc`) for Astro and the test runner.
Playwright is a development dependency
only; it is not required to run the UI.

```sh
npm ci --prefix apps/studio
npm run check --prefix apps/studio
npm run build --prefix apps/studio
cargo test --locked -p rdf-cli --test http
cargo build --locked -p rdf-cli
npm ci --prefix scripts/ui
cd scripts/ui
npx playwright install --with-deps chromium
npm test
# Optional: repeat the workflow through Astro's development proxy.
npm test -- --dev
```

The browser script launches a real server on a free loopback port with a temporary
persistent database. It tests example import, file upload to a named graph,
malformed-import rollback, joined SELECT, ASK false, CONSTRUCT, repeated queries,
raw results and downloads, duplicate/unbound values, safe rendering of HTML-like
literals, pagination, query errors and recovery, mobile overflow, browser history,
and graceful shutdown/reopen. It removes the temporary database and stops the
server afterward. Screenshots are written to `target/ui/desktop.png` and
`target/ui/mobile.png`. `PLAYWRIGHT_MODULE` may point to an existing installation.

The Rust HTTP integration test additionally checks body limits, media types, and
reopening after abrupt server termination, plus plugin discovery and invocation.
The engine's plugin tests run without HTTP. Rust-only builds without a frontend
build return 503 at `/` with build instructions, and retain the full API; the
browser test requires a built Studio. Run the full workspace suite as well:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
```

## Independent correctness

UI and API tests establish application behavior, not standards conformance or an
independent engine oracle. `bash scripts/jena-differential.sh` runs the pinned
Apache Jena comparison corpus; `bash scripts/conformance.sh` runs official pinned
manifest cases. See [core validation status](2026-11-01-oxigraph-validation.md)
for their scope and outstanding release gates. The workbench does not automatically
compare every interactive query with Jena.

The UI is local-only, exposes one dataset per server, and has no account management
or remote deployment mode. Query execution has no forced cancellation. Upload
limits and the four-operation concurrency default apply to UI requests too.

## Frontend development

Run Axum on port 7878, then `npm run dev --prefix apps/studio` in another terminal.
Open http://127.0.0.1:4321/. Astro proxies API requests to Axum; set
`RDF_BACKEND_URL=http://127.0.0.1:OTHER_PORT` before starting Astro to change that
target. Database behavior remains in Rust. Production uses `astro build` output
embedded by `rdf-server/build.rs`; rebuild Rust after rebuilding the frontend.
Both Node dependencies and Rust dependencies have committed lockfiles.

Frontend configuration follows the official [Astro styling guide](https://docs.astro.build/en/guides/styling/)
and [Tailwind Astro integration](https://tailwindcss.com/docs/installation/framework-guides/astro).

## Hot reload

`just dev` starts the Docker UI as an Astro dev server and the backend, then keeps
them in sync (needs `inotify-tools`). Edits under `apps/studio/` are copied into the
UI container and refresh the browser through Vite HMR. Edits under `crates/` rebuild
`target/debug/rdftool` on the host, copy it into the backend container and restart
it; a failed build leaves the previous backend running. `just dev-sidecars` does the
same for running sidecars (Python sync + restart; Jena sources rebuild the image).
Containers keep running after Ctrl+C. `just ui` / `just backend` still build the
production nginx image. Changing `package.json` needs `just dev` restarted.

## Datasets (multi-tenancy)

The Docker backend runs `rdftool gateway`, which keeps one `serve` process per
dataset and routes `/d/<dataset>/...` to it ([ADR 0008](../adr/0008-dataset-isolation-gateway.md)).
Unprefixed paths use the default dataset, so existing clients are unchanged.

- Studio: the sidebar **Dataset** control lists, switches, creates and deletes
  datasets; the **Datasets** page shows status, counts, named graphs, common types and
  properties, and each dataset's plugins. The choice is stored in `localStorage` and
  switching reloads the page.
- API: `GET/POST /api/datasets`, `DELETE /api/datasets/<name>`.
- CLI (database directory, not served): `rdftool datasets --db ROOT list`,
  `create NAME`, `delete NAME --yes`. Delete also removes the dataset's Qdrant
  collections using the semantic config.
- Per-dataset plugin settings: `--tenant-config-dir DIR` with
  `DIR/<dataset>/semantic.json` and `inference.json` overriding the global files.
- Without the gateway (`rdftool serve`), Studio shows one dataset and hides the switcher.

## Graph-first navigation

The front page is a full-page graph explorer; the SPARQL editor is at `/workspace`.
Both pages share a collapsible left sidebar (icon rail when collapsed, remembered
per browser). Clicking a node slides a details drawer in from the right; close it
with the ✕ button or Escape. Graph controls float over the canvas.
Rendering and interaction use D3 7 (`d3-force`, `d3-zoom`, `d3-drag`) on an
SVG canvas. `src/lib/graph.ts` maps RDF terms and hierarchy to plain view
nodes and edges; `D3Graph.tsx` owns geometry, group hulls and the force
simulation (link, many-body, collision and centring forces).
Select a resource to inspect its outgoing properties and incoming relationships.
`rdfs:subClassOf` forms expandable groups: click a class to reveal its
children inside a shaded region around it. `rdf:type` remains an ordinary edge, not class containment.
Multiple inheritance chooses one display parent and keeps other relationships;
cycles do not recursively expand. Nesting is capped at eight levels, with deeper
resources still available in the inspector. No reasoning is performed by the UI.

Use **Try animal example** to append a small, repeatable taxonomy to named graph
`urn:rdftool:examples:animals` and select it. Expand Mammals, then Whale, to see
Elephant and Blue whale. Resource-to-resource properties such as habitat are
labeled edges; literal values appear in the inspector. Search reveals a loaded
node by expanding its ancestors. Drag nodes to move them; the simulation settles again on release. Hover or select a node to highlight its neighbours and show edge labels. Drag the
background to pan, scroll/pinch to zoom, or use Fit graph and Re-layout. Reduced
motion preferences disable animated settling. The accessible node list offers
keyboard selection and class expansion alongside the canvas. Dashed edges
represent connections redirected from a hidden subclass to its visible ancestor.
The inspector retains the original RDF facts.

Each view loads at most 600 statements, with a visible truncation notice. Graph
selection narrows this sample; the inspector explicitly describes loaded facts,
not a complete description of a resource in every dataset graph. All-data mode
omits graphs ending in `:metadata` or `:provenance`; they can be selected explicitly.
Import on the explorer uses the default graph for Turtle/N-Triples and preserves
TriG/N-Quads graph membership. Use the workspace upload dialog to choose a target
named graph explicitly.

Validation: Astro check/build, hierarchy model checks for grouping, cycles and
multiple inheritance, and `scripts/ui/graph-deployed.mjs` against the real Docker
backend. Browser coverage includes properties, nested expansion/collapse, search,
relationships, mobile page sizing, and navigation to SPARQL. Screenshot:
`target/local-deployment/graph-viewer.png`. Browser tests clean up their temporary
graphs. Existing query browser tests now navigate to `/workspace`.

D3 validation additionally exercises real pointer dragging, zoom, force
re-layout and group-hull containment. The simulation is stopped, and the zoom
handlers and ResizeObserver are released, on unmount. The layout is a finite force
simulation that settles (it reheats on drag, expand or re-layout), not a
continuously running animation. See [d3-force](https://d3js.org/d3-force).
