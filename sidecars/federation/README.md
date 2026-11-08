# Virtual graph and federation sidecars

| Service | Image | Port (loopback) | Role |
| --- | --- | --- | --- |
| `postgres` | `postgres:16-alpine` | 15432 | ACME Insurance tables; Ontop connects as the read-only role `ontop_ro` |
| `ontop` | `ontop/ontop-endpoint` (pinned by digest, Ontop 4.2.2) | 18090 | R2RML virtual graph over Postgres, as a SPARQL endpoint |
| `federation` | Node 22 + Comunica 5.4.1 | 18095 | Evaluates `SERVICE` clauses; refuses anything off the allowlist |

```sh
just dev            # Studio + backend (gateway)
just federation     # downloads the JDBC driver, starts and seeds everything, creates the claims/finance datasets
just federation-test
```

`scripts/seed/acme_postgres.py` loads the CSV tables (types from the dataset DDL) and writes
`ontop/mapping.ttl`, the dataset's own R2RML file with table names reduced to the Postgres names.
`mapping.ttl` and `postgresql.jar` are generated or downloaded, so they are git-ignored. Files are copied into
the images rather than bind-mounted so the stack works on Docker Desktop without WSL distro integration.

The local credentials (`rdf-local-only`, `ontop-read-only`) are for loopback development only.

## How a dataset uses them

The backend loads the plugins from per-dataset files in `deploy/local/tenants/<dataset>/`:

- `virtual.json` enables `virtual-graph` (operations `status`, `query`) against the Ontop endpoint.
- `federation.json` enables `federation` (operations `status`, `query`): the Comunica endpoint, the dataset's own
  `/query` URL as `local_source`, and the `allow` list of `SERVICE` endpoints.

The allowlist exists twice on purpose: in the plugin (checked before any request) and in
`comunica/allow.json` (enforced by the sidecar). Keep them equal. SERVICE endpoints are the URLs the
*sidecar* can reach: `http://ontop:8080/sparql` for the virtual graph and
`http://host.docker.internal:7878/d/<dataset>/query` for another dataset.

```sh
curl -s -X POST localhost:4321/d/claims/api/plugins/federation/operations/query \
  -H 'content-type: application/json' -d '{"query": "SELECT ... SERVICE <http://ontop:8080/sparql> { ... }"}'
```

Inference jobs on `default` also read from these sidecars (`deploy/local/tenants/default/inference.json`):
`acme-live` reasons over live Postgres rows and `portfolio-risk` runs rules over a federated join; `risk-notes` is a
hybrid index over the rules' output. [ADR 0012](../../docs/adr/0012-inference-with-outside-inputs.md) explains how.

See [ADR 0010](../../docs/adr/0010-virtual-graph-and-federation-plugins.md).

## Keeping it consistent

- The sidecar's `comunica/allow.json` is generated from `deploy/local/tenants/*/federation.json`
  (`python3 scripts/federation/sync_allowlist.py`; `just allowlist-check` fails when it drifted).
  A dataset's federation `status` reports `misconfigured` if it allows an endpoint the sidecar would refuse.
- `deploy/local/tenants/<dataset>/refresh.json` rebuilds dependent search indexes after an inference job rebuilds.
- Set `FEDERATION_DEBUG=1` to log the requests Comunica sends while diagnosing a federated query. See ADR 0014.
