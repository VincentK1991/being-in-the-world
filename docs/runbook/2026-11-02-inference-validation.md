# Inference validation — 2026-11-02

Validated the new `inference` plugin with the real Docker sidecar using OWL-RL
7.6.2 and RDFLib 7.6.0 on Linux/WSL. The service remains on loopback port 18082.

## Results

- Workspace nextest: **102 passed, 4 skipped** (explicit sidecar/hosted tests).
  An initial HTTP test expected SDK v2; it was updated to v3 and the complete
  regression suite then passed.
- Workspace formatting and all-targets Clippy with warnings denied: passed.
- Generic host test: atomic publication, malformed-output rollback, changed-source
  rejection, dataset isolation, reserved namespace validation, blank-node joins,
  empty output and manual output modification detection: passed.
- Explicit real OWL-RL integration test: class inheritance, inverse properties,
  transitivity, graph isolation, blank-node identity across the sidecar, database
  reopen, stale status, removal of deductions after source deletion, unsupported
  term rejection and preservation of previous output after failed rebuild: passed.
- Six sidecar tests: RDFS, equivalence, symmetry, domain, equality, reported
  inconsistency, unsupported OWL/import terms, malformed inputs and blank nodes:
  passed. These are focused behavioral tests, not a standards conformance suite.
- Real Chromium/Studio workflow: TriG upload, generic HTTP rebuild, SPARQL over
  inferred data, plugin discovery and refreshed graph overview: passed. Existing
  upload rollback, query formats, rendering, mobile and reopen checks also passed.
  Screenshot: `target/ui/inference.png`.
- CLI demo: created `target/inference-demo/db`, built 125 derived triples (including
  vocabulary/equality deductions), and recorded four query results. Assertions are
  stored separately from deductions and generation metadata.

## Actual example outcomes

| Query | Result |
| --- | --- |
| Inferred selected types of car1 | Asset, Vehicle |
| Inverse hasChild relationship | bob → alice |
| office locatedIn country | true |
| excluded secret entity inferred as Vehicle | false |

[Runnable queries and exact results](../../plugins/inference/examples/README.md).
[Setup, configuration and test commands](../../plugins/inference/README.md).

## Boundaries

The implementation declares `rdfs-v1` and `owlrl-core-v1`, not full OWL 2 RL/DL
compliance. Imports, restrictions, class-expression lists and custom rules are
excluded. Rebuilds are explicit. Ordinary SPARQL can read stale inferred graphs;
clients must check status/rebuild. Source publication uses a content fingerprint
and shared supported-API mutation lock; raw Oxigraph store writes bypass that lock.
No large-dataset performance, forced process-crash recovery, or exhaustive OWL
consistency guarantees were established in this run. The timeout and container
resource limits are configured, but timeout/OOM fault injection was not performed.
