# ADR 0015: Chained inference jobs

Status: accepted. Replaces the "derived graphs cannot be sources" restriction from ADR 0006.

## Problem

Rule profiles here run one pass (`sparql-construct-once`), so a rule that needs another rule's or an ontology
reasoner's conclusions needs a second job. Staged reasoning (classify, apply business rules, aggregate) is the usual
shape of production rule systems. Until now a job could only reach another job's output by sending a CONSTRUCT through
the federation sidecar to this same dataset, with no ordering, no declared dependency and no cycle check.

## Decision

- **Reading.** `Dataset` snapshots accept derived graphs as sources; only the `:metadata` bookkeeping graph is refused.
  Nothing else in the engine changed: a derived graph is an ordinary named graph.
- **Declaring.** An inference job depends on another by listing `urn:rdftool:derived:inference:<job>` in
  `source_graphs`. The plugin derives the dependency graph from that, rejects unknown jobs, self-reference and cycles
  when the configuration loads, and keeps every job in dependency order. Other `urn:rdftool:derived:` graphs stay forbidden.
- **Freshness.** The store's source token already covers an upstream graph's content, so a changed upstream output makes
  the downstream job stale. It cannot see an upstream job that is itself out of date and unrebuilt, so `status` also
  asks upstream jobs and reports `stale` with `stale_upstream`.
- **Rebuilding.** `rebuild` takes `cascade` = `upstream` | `downstream` | `both`. Jobs run in dependency order, only
  those that are no longer ready are rebuilt (the named job always is), the first failure stops the run, and the whole
  cascade holds the single rebuild slot. The response carries `cascade.steps`; other rebuilt jobs are listed in `ran`.
- **Refresh rules.** The engine treats each entry of `ran` as one more call of the same operation when matching refresh
  rules, so an index refresh written for a middle job fires when a cascade rebuilt it.
- **Lineage.** A new `lineage` operation returns upstream and downstream jobs with states and the rebuild order.

## Consequences

- Provenance is unchanged and now useful: a job's recorded `graphs` include the upstream output it read, so a SPARQL query
  over the metadata graphs shows the chain (workspace: "Which jobs read other jobs' output").
- A cascade takes as long as its slowest path of jobs because they run one after another.
- A skipped job is judged by `status`, which re-reads outside inputs, so a cascade makes those requests too.
- Jobs only see the graphs they list. A downstream job usually lists the asserted graphs as well as the upstream output,
  because OWL-RL and rule outputs contain only what they added.
- Still not done: running independent jobs in parallel, scheduled refresh, and per-triple provenance.

## Alternatives

- Reading upstream output through a federation input needs no engine change, but has no ordering, cycle detection or
  staleness through the store. Kept for outside data, where it belongs.
- Letting a job list its own output (iteration to a fixpoint) was rejected: the source token changes on every pass, so
  it cannot be told apart from drift. Fixpoint reasoning stays inside a reasoner (OWL-RL).
