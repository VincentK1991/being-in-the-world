# Embedding provider validation — 2026-11-02

## Implemented

- Provider SDK (`rdf-embedding-api`) and independent built-in adapter crate
  (`rdf-embedding-providers`). The semantic plugin has no production dependency
  on adapters or provider-specific APIs.
- Per-index named provider configuration; local CPU/GPU and OpenAI-compatible
  adapters; environment-only credentials; backward parsing of legacy local config.
- Generic vector-space identity and schema 2 manifests. Existing schema 1 vector
  indexes require a rebuild; RDF is unchanged.
- Eight executed SPARQL examples and a ninth opt-in mixed local/OpenAI query.

## Evidence

- `cargo nextest run --workspace --all-targets --offline`: **101 passed, 3 skipped**.
  The skipped tests require explicit real sidecar/hosted service invocation.
- `cargo fmt --all --check`, workspace/all-targets Clippy with warnings denied,
  and `git diff --check`: passed.
- Explicit real Qdrant + CUDA integration test
  `semantic_search_rebuild_staleness_isolation_and_reopen`: passed.
- Explicit `mixed_indexes_with_openai_contract_fixture`: passed. Used real Qdrant,
  real local model, and a local HTTP fixture for the hosted endpoint. Verified
  distinct vector dimensions in simultaneous indexes, SPARQL UNION routing,
  revision mismatch rejection after reopen, and unaffected local index readiness.
- OpenAI contract tests: passed. Request model/dimensions/float encoding/auth,
  response reordering, wrong model, duplicate indexes, wrong dimensions, zero
  vectors, HTTP 401/429, input length policy and missing credentials.
- Third-party factory test: passed without changes to semantic-search routing.
- Chromium `scripts/ui/smoke.mjs --semantic`: passed with named provider config,
  including Turtle upload, HTTP rebuild, semantic SPARQL and shutdown/reopen.
- Eight CLI examples: executed with the real 384-dimensional MiniLM model on
  RTX 4090/CUDA. [Queries and results](../../plugins/semantic-search/examples/README.md).
  The retained local demo database/results are in `target/semantic-provider-demo`.

`OPENAI_API_KEY` was absent. The real hosted test
`local_and_openai_indexes_in_one_query` was **not executed**. OpenAI relevance,
account access, billing and live latency are not established by contract fixtures.
No actual hosted result is included in the recorded examples.

The GPU sidecar and Qdrant were already running and remain available on loopback
ports 18081 and 16333. This run did not modify the model service or retest CPU
inference; CPU validation of that service is recorded in the earlier semantic
validation runbook.

## Reproduce

See the [provider setup and test commands](../../plugins/semantic-search/README.md#multiple-embedding-providers)
and [architecture decision](../adr/0005-embedding-provider-extensions.md).
The hosted test requires a configured API key and explicitly sends its small test
fixture corpus to OpenAI. Never put the key in the checked-in JSON configuration.
