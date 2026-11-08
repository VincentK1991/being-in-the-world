---
updated_at: 2026-11-08T15:00:00Z
status: in_progress
---

# Live stack in CI

`.github/workflows/live-stack.yml` runs nightly (03:17 UTC) and on demand. It calls
`scripts/ci/live-stack.sh`:

1. **Build:** `rdftool`.
2. **Federation sidecars:** Postgres (seeded with ACME), Ontop and Comunica.
3. **Reasoner sidecars:** OWL-RL and Jena.
4. **Semantic search:** Qdrant and CPU embeddings. The first start downloads the model.
5. **Backend and Studio.**
6. **Seed:** `local-demo.py`, `seed/acme_insurance.py` and `seed/federation_demo.py`.
7. **Test:**
   - `scripts/test/rule_tests.py`;
   - `scripts/test/federation_e2e.py`;
   - `scripts/ui/inference-deployed.mjs`, the Studio inference panel in Chromium;
   - `scripts/conformance.sh` with the reasoner sidecars.
8. **Upload:** container logs (`target/ci-logs`), Studio screenshots (`target/ui`) and conformance
   reports, whatever the outcome.

A single phase can be run with `bash scripts/ci/live-stack.sh up | seed | test | logs`.

## Networking

The local stack relies on Docker Desktop resolving `host.docker.internal` to ports the host
publishes on `127.0.0.1`. A Linux runner's bridge gateway cannot see the host's loopback, so the
job publishes on `0.0.0.0` (`RDF_PUBLISH_HOST`), and the backend maps `host.docker.internal` to
`host-gateway`. Locally nothing changes: every port still defaults to `127.0.0.1`.

## Verified so far

- **Compose:** with the variable unset, every Compose file renders loopback-only ports. With
  `RDF_PUBLISH_HOST=0.0.0.0` (passed through `WSLENV` from WSL), they render `0.0.0.0`. The
  backend carries `host.docker.internal=host-gateway`, and the local stack still passes everything
  with that mapping.
- **Phases against the running local stack:**
  - `seed` (`local-demo.py`, `federation_demo.py`) succeeds.
  - `test` exits 0: rule tests 15/15, end to end 72/72, Studio panel 10/10, and all seven
    conformance baselines match. The end-to-end suite takes about 5 minutes, mostly federated
    queries.
- **Bug found by the rehearsal:** loading every inference job's state took 8–60 s, because each
  status re-ran the jobs' outside-input queries, and the federated one sometimes hit Comunica's
  60 s timeout. `status` now takes `"inputs": false`. The Studio overview uses it (0.4 s) and
  checks the selected job's inputs on their own.

## Not yet verified

- A cold start on a GitHub runner, meaning the `up` phase from an empty machine. Locally that
  would mean stopping the shared development stack.
- `seed/acme_insurance.py` from a fresh store. Locally rdflib is not installed, and the ACME graph
  was already present.

The first scheduled or manual run of the workflow settles both. On failure, check the uploaded
container logs first.
