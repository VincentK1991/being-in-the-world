# ADR 0014: Federation reliability and derived-index freshness

Status: accepted. Follows ADR 0010 to 0013; fixes the limits those records listed.

## Decisions

1. **One source for the allowlist.** `deploy/local/tenants/<dataset>/federation.json` is the source. The
   sidecar's `allow.json` is generated from the union of those files by
   `scripts/federation/sync_allowlist.py`; `just allowlist-check` (part of `just ci`) fails when it drifted, and
   `just federation` regenerates it. The federation plugin's `status` asks the running sidecar for its list and reports
   `state: "misconfigured"` with `missing_in_sidecar` when this dataset allows something the sidecar would refuse.
2. **Derived indexes refresh themselves.** `deploy/local/tenants/<dataset>/refresh.json` holds rules
   `{after: {plugin, operation, input?}, then: {plugin, operation, input?}}`; the gateway passes the file to the
   dataset server (`RDF_REFRESH_CONFIG`). After a matching operation succeeds, the engine runs `then` and reports it in
   the response as `refreshed` (including a failure, which does not fail the original operation). Rules may not chain.
   Example: after `inference rebuild {job: portfolio-risk}`, run `semantic-search rebuild {index: risk-notes}`.
3. **Transient "sidecar unavailable".** Cause: Node closes idle connections after 5 s while the Rust client kept pooled
   ones for 90 s, so a request could reuse a connection the server had just closed. The plugin's client now drops idle
   connections after 2 s and retries a failed (not timed-out) read-only request once; the sidecar keeps connections for
   65 s. Errors now say which sidecar was unavailable.
4. **Property calls survive pushed-down filters.** Comunica forwards a `FILTER` as a group around one list cell, which
   split a call's list and failed with "property lists must be finite". The engine merges the two triple blocks back
   when the result contains a property call and every filter only uses variables its own block binds.
5. **`GRAPH` inside `SERVICE`.** The sidecar wraps such bodies in `SELECT * { ... }` (Comunica ignores a bare `GRAPH`).
6. **Misleading counter removed.** Comunica's `httpRequests` is a process-wide total, so the sidecar strips it from results.
   Set `FEDERATION_DEBUG=1` on the sidecar to log outbound requests while diagnosing.

## Consequences

- A rebuild of a refreshed job takes as long as the index rebuild; the request returns when both finish.
- Refresh rules are per dataset and read at start, like other plugin configuration.
- The sidecar rewrites queries it forwards only for the `GRAPH` case; everything else is passed through as written.
