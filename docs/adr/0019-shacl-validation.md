# ADR 0019: Explicit SHACL validation and severity gates

Status: accepted. User-selected default: report-only, with explicit validation gates.

## Decision

Implement `plugins/shacl` against the existing SDK v3. Apache Jena 6.2.0's
`jena-shacl` module evaluates SHACL Core in the existing bounded Jena subprocess
sidecar. RDF storage, SPARQL execution, and the HTTP backend remain independent.
No custom SHACL evaluator or database-specific SHACL routes are added.

Each dataset may configure jobs with explicit data graphs, shapes graphs, and a
gate severity threshold. Users upload ordinary Turtle shapes through Graph Store
or Studio. Data and shape graphs are disjoint; each role is merged independently.
There is no automatic inference. Include a chosen inference output graph when
needed and check its freshness separately.

Operations:

- `jobs`: configuration and declared profile.
- `validate`: produce and atomically replace the job's standard RDF validation
  report, returning findings and a summary. Findings do not fail this operation.
- `check`: perform the same validation and publication, then fail the operation
  if any finding meets the job's threshold. The saved report remains available.
- `status`: report missing/ready/stale, including the previous summary.

`sh:severity` sets each constraint's severity: Violation (default), Warning, or
Info. The configured `fail_on` threshold is independent of `sh:conforms`:
`violation` rejects Violations; `warning` rejects Warnings and Violations; `info`
rejects all three. No conformance flag is rewritten to implement policy.

The host publishes reports at `urn:rdftool:derived:shacl:JOB` and source/config
metadata at its `:metadata` graph. Reports are queryable with standard SPARQL.
SDK source tokens detect changes during computation and publication. Since SDK
v3 merges snapshots, bracket separate data/shape snapshots with the combined
source token; publication rechecks that token under the host mutation gate.
Failed providers, malformed shapes, oversized output, or source races preserve
the last complete report. Shape changes, data changes, report tampering, and job
policy changes make its status stale. Only the latest report is retained.

## Explicit boundaries

An explicit check gates a workflow's next step; it does not intercept uploads,
SPARQL updates, Graph Store mutations, inference publication, or bulk ingestion.
The generic operation API returns HTTP 400 on a failed check; the CLI exits
nonzero. Report-only findings return HTTP 200. Provider errors also fail the
operation, with their own messages; they never masquerade as violations.

A check followed by a write is not atomic. Future strict write enforcement needs
a generic host pre-commit validator contract, validation of the proposed state,
and defined behavior for all mutation paths. Validating after committing and
calling it rejection would be incorrect. Quarantine/promotion, incremental
validation, delta-only gates, and automatic validation scheduling are deferred.

The initial profile rejects SHACL-SPARQL, JavaScript, rules, custom validators,
external targets, explicit entailment, and imports. Shapes and data never load
remote documents. An empty shapes graph fails; empty data or unmatched targets
can conform under normal SHACL semantics. We do not claim full SHACL conformance
from the included behavioral tests or comprehensive shapes-graph linting.

Limits: SDK snapshots 16 MiB each; 100,000 triples per role in Jena; 1,000 top-level
findings with no truncation; bounded report/HTTP output; one Jena worker at a
time, 384 MiB JVM heap and 60-second subprocess timeout. Rust requests time out
after 70 seconds. No caller-controlled provider endpoint or job definition is
accepted in operations; configuration is trusted, loaded at startup per dataset.

## References and verification

- [Apache Jena SHACL](https://jena.apache.org/documentation/shacl/)
- [W3C SHACL](https://www.w3.org/TR/shacl/)
- [Usage, policies, and validation commands](../../plugins/shacl/README.md)
