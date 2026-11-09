# Studio documentation site (Astro Starlight)

Status: proposed implementation specification, 2026-11-08, written from an interview. Baseline commit `e94d66a`. Nothing here is built yet.

## Overview

Add a documentation site to RDF Studio, served at `/docs`, built with Astro Starlight. It has three sections:

1. **What it is.** An RDF store built on Oxigraph, with a plugin architecture for add-ons.
2. **How to use.** How to install it, use it, and what each part does.
3. **Internals.** How it works, in enough detail for a developer to operate and extend it: the request path, storage and indexes, the plugin architecture, the vector index, inference, federation and virtualization.

The reader is an engineer. Every Internals page serves three needs: understanding how it works, operating it, and extending it.

The text is new, written for the site. ADRs stay the record of decisions, and the site links to them. It does not copy them.

## Goals and non-goals

**Goals**

- One place where a developer can learn the whole system, from install to internals.
- Statements that stay true: build-time and CI checks catch links, file paths, commands and stale pages.
- Honest about limits, including no authentication and loopback-only binding.

**Non-goals**

- Importing the plugin READMEs or ADRs into Starlight (D2).
- A separate Reference section. Generated tables sit inside the pages that need them (D8). A Reference section can be added later without restructuring.
- Versioned docs or translations. English only, one version.
- Changing the backend. Docs are static files, and the nginx change is only routing for `/docs`.

## Constraints from the codebase

These come from the Phase 1 analysis and drive several design choices below.

- **Astro 7.4.4**, `output: 'static'`, React islands, Tailwind v4. `@astrojs/starlight` 0.42.10 declares `astro ^7.2.10`, so it is compatible. Starlight is pre-1.0 and its API still moves, so pin it exactly.
- **Studio pages import `global.css` one by one.** Tailwind therefore does not apply to Starlight pages unless one of them imports it. Keep it that way, and check it in the browser test.
- **The UI image copies only `apps/studio/`** (`deploy/local/ui.Dockerfile`, then `npm run check && npm run build`, then nginx). Anything that needs files elsewhere in the repo cannot run inside that build. It runs on the host (`just docs-check`) and in CI.
- **The dev proxy and nginx route `^/d/`, `/api`, `/query`, `/update`, `/store`, `/health` and `/metrics` to the backend.** `/docs` does not match any of them. No docs slug may start with `d/`.
- **Pagefind** (Starlight's search) ships a platform binary through npm. It must build on `node:22.23.4-bookworm-slim`. Phase 1 proves that by building the image.

## Information architecture

All pages live under `/docs/`. Each Internals page has the same three parts after a short at-a-glance box: **How it works**, **Operating it** and **Extending it**. It ends with **Sources**, the list of repo files the page describes.

### 1. What it is

| Page | Must cover |
|---|---|
| Overview | What the store is, why it is built on Oxigraph, what the plugin architecture buys, who it is for. |
| Architecture | One diagram of the whole system: Studio, nginx, gateway, dataset servers, engine, Oxigraph, plugins, sidecars. |
| Components | A table of crates, plugins and sidecars with their ports and what each owns. Generated (D8). |
| Status and limits | Candid list of known gaps: no authentication, quotas or idle shutdown, OWL coverage numbers from `docs/conformance.md`, Oxigraph literal normalisation, what is not built. Required by D10. |

### 2. How to use

| Page | Must cover |
|---|---|
| Install | Three tiers (D7): core CLI and server; Docker backend and Studio; then each sidecar one at a time. Every tier ends with an "is it working" check. Carries the security warning (D10). |
| CLI | `ingest`, `query`, `update`, `query-pattern`, `dump`, `stats`, `backup`, `restore`, `datasets`, `gateway`, `serve`, `plugin`, `test-suite`. |
| Studio tour | Graph explorer, SPARQL workspace, Datasets, Plugins, the Inference panel, the SHACL panel. |
| HTTP API | `/query`, `/update`, `/store`, `/api/plugins/...`, `/d/<name>/...`, the SPARQL Service Description, error codes (503 capacity). |
| Datasets | Isolation, the catalog, the gateway, per-dataset configuration under `tenants/`. Carries the security warning. |
| Using the plugins | Search, inference, SHACL, federation, virtual graph, plugin catalog: what each does and the shortest working example. |

### 3. Internals

| Page | Phase | Must cover |
|---|---|---|
| Request path | 1 | Browser to nginx to gateway to a dataset server to Axum to the engine to Oxigraph. Query: parse, policy check, property-function rewrite, execute. Update: validation and the mutation lock. Operation permits and 503. This page also covers query planning, so there is no separate "indexes and query engine" page. |
| Storage | 1 | The RocksDB layout in Oxigraph 0.5.13: column families, term encoding, the permuted indexes and when each is used. Our layer: dataset catalog with UUID directories, derived graphs and their metadata graph, attempt records, redb migration, backup and restore. Verified against the pinned source (D4). |
| Plugin architecture | 1 | `rdf-plugin-api` (`Plugin`, `ReadContext`, `Facet`, `CatalogEntry`, SDK version), registration order, property functions and the query rewrite, refresh rules, the catalog and Service Description, provider registries for reasoners and embeddings. Extending: how to write a plugin. |
| Vector index | 1 | The semantic-search plugin: embedding providers, Qdrant collections and how rebuilds publish a new collection, payloads, BM25 term vectors, hybrid ranking, freshness and what backup does not include. |
| Inference | 2 | Jobs, derived graphs and their lifecycle, providers (SPARQL, Jena, owlrl, native OWL 2 RL), chaining and cascades, clashes, rule tests, explanations. |
| Federation and virtualization | 2 | Comunica `SERVICE` federation, the allowlist, reliability, the Ontop virtual graph over Postgres, query-time reasoning, discovery through the catalog. |
| Gateway and isolation | 2 | One server process per dataset, routing, tenant configuration, environment handed to children. |
| Testing and conformance | 2 | Test layers, the W3C suites and baselines, the nightly live-stack job. |

## Technical design

### Site layout

- Add `@astrojs/starlight` (pinned) and `starlight-links-validator`. Register Starlight in `apps/studio/astro.config.mjs`. Keep the React integration and the Vite proxy.
- Content under `apps/studio/src/content/docs/docs/<section>/<page>.md(x)`, so routes are `/docs/<section>/<page>/`. Add `src/content.config.ts` with Starlight's docs collection. Confirm the exact 0.42 wiring while building Phase 1.
- Starlight sidebar has the three sections in order, plus a top link back to Studio (`/`).
- Studio's `AppSidebar` gets a **Docs** item linking to `/docs/`. The docs header links back to Studio.
- Theme: `customCss` maps Starlight's accent colours to Studio's palette (`#24785b` accent, `#14352b` forest), with a lighter accent in dark mode. Check contrast for text and links in both modes.

### nginx

Add a `location /docs/` that serves files, then directories, then a 404 using Starlight's `404.html`. Today the catch-all returns Studio's `index.html` for any unknown path, which would show the graph explorer for a bad docs URL. Pagefind writes its index to `/pagefind/` at the site root, which the existing static `location /` already serves.

### Diagrams

Hand-authored SVG files checked into the repo (D6). Rules:

- One consistent style: the same palette as Studio, shapes and arrows from a shared small set.
- Colours come from CSS variables so diagrams follow dark mode.
- Each SVG has `<title>` and `<desc>`, and the page text states what the diagram shows, so nothing depends on seeing it.
- Text in SVG is real text, not paths, so it can be searched and selected.
- Expect 10 to 15 diagrams across both phases. At least: system architecture, request path, storage layout, plugin hooks, vector index lifecycle in Phase 1.

### Accuracy mechanisms (D8)

| Guard | What it does | Where it runs | Phase |
|---|---|---|---|
| Link and path check | `starlight-links-validator` fails the build on a broken internal link. A `<Src file="..." symbol="..."/>` component renders a link to the file on GitHub, and `scripts/docs/check_paths.py` fails if the file or symbol does not exist. | Links: `npm run build`, so inside the image. Paths: host and CI only (`just docs-check`, part of `just ci`, and a step in the Rust foundation workflow), because the image does not contain the repo. It is Python, like the repo's other scripts, so no Node is needed on the machine that runs it. | 1 |
| Verified-against stamp | Every Internals page has `verified: { commit, refs, oxigraph }` in frontmatter. `scripts/docs/verify.py` lists pages whose `refs` changed since `commit`. Re-reading a page and updating its stamp is the act of re-verifying. Storage's stamp includes the Oxigraph version, so an Oxigraph upgrade flags it. | Host and CI. | 2 |
| Executable examples | Code fences tagged for execution run against the live stack in the nightly job, in page order within each install tier. A wrong flag or command fails the nightly run. | `scripts/ci/live-stack.sh test`. | 2 |
| Generated tables | `scripts/docs/snapshot-catalog.py` writes `apps/studio/src/generated/plugins.json` from `GET /api/plugins` and the Service Description. Tables for components, plugin operations and SPARQL properties render from it. The nightly job fails if the committed snapshot differs from the live catalog. | Script on the host; check in nightly. | 2 |

Phase 1 uses `<Src>` and links from the start, so nothing has to be retrofitted. The stamp, example and table fields are optional in Phase 1 pages and required in Phase 2.

### Storage page: verification source

Oxigraph 0.5.13 is in the local cargo registry (`oxigraph-0.5.13/src/storage/rocksdb.rs`). Its column families are declared there (`id2str`, `spog`, `posg`, `ospg`, `gspo`, and the `d*` and `graphs` families). Every statement about them is checked against that file. The page says "Oxigraph 0.5.13", and the Oxigraph upgrade checklist (already on the roadmap) includes re-verifying it.

## Security considerations

The docs are static files and add no backend endpoint or write path. The risk is what they tell people to do. The store has no authentication, so the documentation must not lead anyone to expose it. Acknowledged and resolved in D10.

- **Warning box** on Install, Datasets and the HTTP API page: there is no authentication, the server binds to loopback only, and `RDF_PUBLISH_HOST=0.0.0.0` is for throwaway CI runners, never a developer machine or a shared host.
- **Status and limits** lists the missing pieces: authentication, quotas, idle shutdown.
- **Demo credentials** (for example the Postgres password in the federation compose file) are shown only labelled as local-only demo values. No real secret, token or key appears anywhere in the docs. Executable examples use only the demo stack.
- The Installation tiers do not suggest port-forwarding or reverse-proxying the store.
- Pagefind and the site make no network calls to third parties. No analytics.

Hard-block check: authentication/authorization bypass risk is a property of the product, not of the docs. The user chose to document it prominently (D10). Runtime guard rails (refusing a non-loopback publish) are a separate code change, not part of this spec.

## Testing strategy

- **Build gate.** `npm run check && npm run build` in the image. Fails on type errors, content schema errors and broken links.
- **`just docs-check`** (host, in `just ci`): path and symbol check; in Phase 2 also the stamp check.
- **Browser test** `scripts/ui/docs-deployed.mjs`, added to the nightly live-stack run:
  - `/docs/` loads, with the three sections in the sidebar.
  - Search finds a known term (proves Pagefind built and is served).
  - The Studio sidebar's Docs link works, and the docs "back to Studio" link works.
  - Diagrams render, and each has a title.
  - No horizontal page scroll at 390 px.
  - Dark mode toggle changes the theme.
  - A bad `/docs/...` URL returns the docs 404, not Studio.
  - Studio pages still render with Tailwind (no style leak either way).
- **Image build** of `deploy/local/ui.Dockerfile` succeeds with Starlight and Pagefind. Recorded in Phase 1 acceptance.

## Edge cases and risks

- **Pagefind or image build fails in `node:slim`.** Found in Phase 1 before any content is written. Fallback is to fix the image, not to drop search.
- **Starlight API churn (pre-1.0).** Pin the exact version; upgrades are deliberate.
- **Trailing slash.** `/docs/x` without a slash gets nginx's directory redirect. Check it works behind the real port mapping.
- **Hand-drawn SVG drifts.** The Sources list and stamps cover the text. Diagrams are only covered by the page's stamp, so review them when the stamp is refreshed.
- **Doc volume.** About 20 pages with three tracks each is large. Phasing (D9) exists to keep Phase 1 shippable.
- **ADR links.** The repo has a GitHub remote, so `<Src>` and ADR references link to `https://github.com/VincentK1991/being-in-the-world/blob/main/<path>`. Links to `main` can move; the path check catches renamed files.
- **Stale root README.** It says reasoning, vectors and virtual SQL are deferred. Fix it and point it at the docs in Phase 1.

## Open questions

These were not decided in the interview. Resolve them while building, and log the answers.

1. Should a stale stamp fail CI immediately, or warn for some number of days? The spec says `just docs-check` reports it. Severity is open.
2. Exact syntax for tagging executable code fences, and how expected output is stated.
3. The interview's page list had a separate "Indexes and the query engine" page. This spec folds it into Request path (planning) and Storage (indexes). Revert if that makes either page too long.
4. Whether `<Src>` links point at `main` or at the stamp's commit.

## Decisions Log

| ID | Topic | Decision | Rationale | Source | Date |
|---|---|---|---|---|---|
| D1 | Audience and depth | Every Internals page serves three needs: understanding, operating, extending, as separate parts | The reader may want any of them; separate parts let them stop where they need to | Interview | 2026-11-08 |
| D2 | Content source | New Starlight pages that link to ADRs. READMEs and ADRs are not imported | The user wants a guided narrative. Facts live in two places, so D8 guards are required | Interview | 2026-11-08 |
| D3 | Structure | Three sections: What it is, How to use, Internals. A fourth Reference section was offered and not chosen | Matches the request. Generated tables sit inside pages | Interview | 2026-11-08 |
| D4 | Storage depth | Document Oxigraph's own storage internals, verified against the pinned 0.5.13 source | It is most of "how it stores data". Version-stamped so an upgrade forces a re-check | Interview | 2026-11-08 |
| D5 | Placement | Inside Studio at `/docs`, linked from the Studio sidebar, with a link back | One build, one container, shared navigation | Interview | 2026-11-08 |
| D6 | Diagrams | Hand-authored SVG, themed with CSS variables | No browser needed in the image, works without JavaScript, full control of layout | Interview | 2026-11-08 |
| D7 | Install scope | Tiered: core, then Studio, then each sidecar, each with a check | Readers stop at the tier they need | Interview | 2026-11-08 |
| D8 | Accuracy guards | All four: link and path check, verified-against stamps, executable examples, generated tables | D2 and D6 make silent drift the main risk | Interview | 2026-11-08 |
| D9 | Phasing | Phase 1: scaffold, theme, link and path check, What it is, How to use, and the Request path, Storage, Plugin architecture and Vector index pages. Phase 2: the remaining Internals pages, stamps, executable examples, generated tables | Something useful ships early. The user took the recommended phasing over a single pass | Interview | 2026-11-08 |
| D10 | Security posture | Prominent warning on Install, Datasets and HTTP API, plus a Status and limits page. Demo credentials labelled local-only. Runtime guard rails are out of scope | The store has no authentication, and the docs must not lead anyone to expose it | Interview | 2026-11-08 |
| D11 | Defaults | Starlight light and dark with Studio's accent, Pagefind search, English only, no versioning, the same three-part page shape, `npm run check && npm run build` stays the gate, and the stale root README is fixed | Stated to the user, who did not object | Interview | 2026-11-08 |
| D12 | Process | Red-team pass skipped. Artifact preview skipped | The user chose to go straight to the markdown spec | Interview | 2026-11-08 |

## Dependency Graph & Implementation Order

```
                 ┌─────────────────────────────┐
                 │ 1. Scaffold + image proof   │  Starlight, Pagefind, nginx /docs, theme
                 └──────────────┬──────────────┘
                                │
          ┌─────────────────────┼──────────────────────┐
          ▼                     ▼                      ▼
 ┌─────────────────┐  ┌───────────────────┐  ┌──────────────────────┐
 │ 2. Studio link  │  │ 3. <Src> + path   │  │ 4. SVG style kit     │
 │    + browser    │  │    check + links  │  │    (palette, shapes) │
 │    test         │  │    validator      │  │                      │
 └────────┬────────┘  └─────────┬─────────┘  └──────────┬───────────┘
          │                     │                       │
          └─────────────────────┼───────────────────────┘
                                ▼
                 ┌─────────────────────────────┐
                 │ 5. Phase 1 content          │
                 │   What it is · How to use   │
                 │   Request path · Storage    │
                 │   Plugin arch · Vector idx  │
                 └──────────────┬──────────────┘
                                ▼
                 ┌─────────────────────────────┐
                 │ 6. Root README fix, runbook │  Phase 1 done
                 └──────────────┬──────────────┘
                                ▼
        ┌───────────────────────┼────────────────────────┐
        ▼                       ▼                        ▼
 ┌──────────────┐   ┌─────────────────────┐   ┌───────────────────┐
 │ 7. Phase 2   │   │ 8. Stamps + verify  │   │ 9. Catalog snapshot│
 │    pages     │   │    script           │   │    + table component│
 └──────┬───────┘   └──────────┬──────────┘   └─────────┬─────────┘
        └──────────────────────┼────────────────────────┘
                               ▼
                 ┌─────────────────────────────┐
                 │ 10. Executable examples in  │
                 │     the nightly job         │
                 └─────────────────────────────┘
```

Order and acceptance:

1. **Scaffold and image proof.** Starlight and Pagefind build inside `deploy/local/ui.Dockerfile`; `/docs/` served by nginx; 404 handled; Studio pages unchanged. *Done when* the image builds and `/docs/` and a search both work from the container.
2. **Studio link and browser test.** Docs item in `AppSidebar`, link back, `docs-deployed.mjs` in the nightly run.
3. **`<Src>`, path check, link validator.** `just docs-check` in `just ci`.
4. **SVG style kit.** A shared palette and shape set, and one finished diagram as the pattern.
5. **Phase 1 content.** Pages as in the tables above, each with a Sources list. Storage claims checked against the Oxigraph source.
6. **README fix and runbook entry.**
7. Phase 2 pages, **8.** stamps and `verify.py`, **9.** catalog snapshot and table component, in any order.
8. **Executable examples,** last, because they depend on stable page content.
