# Rule-provider validation — 2026-11-02

Implemented native one-pass SPARQL CONSTRUCT rules and stateless Jena custom rules
under the existing inference plugin. Provider selection is per job. No database
engine SDK or inference-specific HTTP handler changes were required.

## Results

- Workspace nextest: **107 passed, 5 skipped** (explicit external-service tests).
- Workspace formatting, all-targets Clippy with warnings denied, build and diff
  whitespace checks: passed.
- Five native SPARQL tests: original-snapshot one-pass semantics, deduplication,
  arithmetic/filter/OPTIONAL, source blank-node joins, network/dataset-clause and
  non-CONSTRUCT rejection, output limits, rule-change invalidation, malformed-rule
  preservation, deletion/rebuild and persistence: passed.
- Real Jena + native SPARQL integration: passed. Both derived countries; Jena also
  chained continent facts while one-pass SPARQL did not. Excluded graphs remained
  excluded. Blank-node joins, stale status, failed-rule preservation, deletion and
  reopen behavior passed.
- Four Jena service test methods (including multiple rejected-rule cases): passed.
  Covered recursive closure, language/datatype lexical fidelity, source blank-node
  identity, imports, procedural built-ins, negation built-ins, backward rules,
  unbound head variables, malformed syntax and unknown profiles.
- Existing real OWL-RL integration: passed using legacy single-endpoint config.
- Real Chromium/Studio workflow: passed after correcting the test viewport setup.
  Uploaded TriG, rebuilt all three example jobs through generic HTTP operations,
  queried both providers, and refreshed derived graph listings. Screenshot:
  `target/ui/rules.png`. Existing baseline UI checks also passed.
- CLI demo retained at `target/rule-provider-demo/db`, with exact results alongside.
  [Queries and results](../../plugins/inference/examples/rules/README.md).

An initial SPARQL blank-node test failed because generic import helpers intentionally
rename blank nodes. Snapshot replay now parses and inserts directly into the private
store, preserving identity; the regression passes.

## Observed cancellation limitation

Stress probes using large VALUES joins (with/without aggregation) did not promptly
observe the engine cancellation token. Those owned probe processes were terminated;
they are not reported as passing timeout tests. The unsupported hard-deadline
expectation was removed from the automated suite. Configuration is explicitly
`cancel_after_seconds`, a cooperative request, not a hard timeout. The provider
also checks the token while collecting output and before returning; once control
returns, overdue results fail rather than being published. Eager computation can
still delay that return, and intermediate memory is not hard bounded.

Jena executes in a separate Java worker with a configured 60-second process timeout
and JVM/container memory limits. This run did not fault-inject timeout/OOM behavior.
Neither performance benchmarks nor exhaustive rule-language/standards conformance
are established by the behavioral suite.

## Runtime and reproduction

Jena 6.2.0 / Java 21 sidecar remains running at `http://127.0.0.1:18083`.
OWL-RL remains on 18082; the SPARQL provider requires no sidecar. Jena uses no
persistent database volume. The live database remains Oxigraph.

See [provider instructions](../../plugins/inference-providers/README.md) and
[architecture decision](../adr/0007-inference-rule-providers.md) for exact commands,
profiles, restrictions, freshness and rule configuration.
