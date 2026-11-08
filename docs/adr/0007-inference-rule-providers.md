# ADR 0007: Rule providers under the inference plugin

Status: implemented, 2026-11-02.

The inference plugin owns job orchestration, source snapshots, freshness and atomic
publication. Independently registered providers own rule execution. This extends
ADR 0006 without changing the engine SDK or adding plugin-specific backend routes.

Config now supports a provider map and per-job provider/rule selection. Legacy
single-endpoint OWL-RL config remains readable. Job and selected-provider config,
including rule text, are fingerprinted. Other providers' config does not invalidate
an unrelated job. Existing pre-provider output needs a one-time rebuild because
fingerprint representation changed. No asserted RDF migration is performed.

`rdf-inference-providers` provides the application registry. Its native SPARQL
adapter creates a private in-memory Oxigraph-backed Dataset and reuses the existing
parsed local-query policy. It does not access the live database. Every CONSTRUCT
reads the original merged source graph once; rule results are unioned, deduplicated
and stripped of asserted facts. There is no implicit recursion or rule ordering.
Snapshot replay uses the parser plus transaction inserts rather than the general
import helper, because general import intentionally renames blank nodes.

The Jena provider uses the existing HTTP reasoner protocol extended with a `rules`
array. The stateless sidecar contains Jena 6.2.0 libraries / Java 21, not Fuseki or
TDB. A fresh Java worker executes each job; a lightweight HTTP process remains
running. This favors enforceable worker termination over JVM warm-start latency.
A warm JVM pool is a later optimization, not implemented here.

Jena forward RETE rules are restricted to positive triple-pattern clauses with
range-restricted heads. Recursive closure is supported, but procedural built-ins,
imports, fresh entities and backward/nested rules are not. It is not an OWL profile.
Blank-node and literal identities survive the exchange.

The host's existing atomic publication and source-content check remain unchanged.
Failure retains previous output; status identifies stale data/config. Ordinary
SPARQL can still inspect stale output, and source changes require explicit rebuild.
Provenance remains generation-level; per-fact rule explanations are deferred.

SPARQL cancellation is cooperative. Actual stress testing demonstrated that eager
VALUES joins and aggregates can run without promptly observing the engine token.
We therefore expose `cancel_after_seconds`, not a hard timeout claim. Checks during
result collection and before return reject overdue results once control returns.
No hard intermediate-memory or CPU isolation exists for the embedded provider.
Jena worker deadlines terminate a process; container/JVM memory limits bound it.

Tests include real Jena/native parity, recursive versus one-pass behavior, arithmetic,
blank-node joins, literal identity, graph scope, deletion/rebuild, rule-change
invalidation, bad-rule rollback, output limits, disabled network features, browser
upload and backend invocation. They establish adapter behavior, not general Jena,
SPARQL, or OWL standards conformance.

Sources:
- [Jena rule engine](https://jena.apache.org/documentation/inference/)
- [Jena releases and Java requirements](https://jena.apache.org/download/)
- [Jena blank-node label policy](https://jena.apache.org/documentation/javadoc/arq/org.apache.jena.arq/org/apache/jena/riot/lang/LabelToNode.html)
