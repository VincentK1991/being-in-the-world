# ADR 0006: Inference and host-managed derived graphs

Status: implemented, 2026-11-02.

The first inference provider is a Docker sidecar wrapping pinned RDFLib OWL-RL
7.6.2 / RDFLib 7.6.0. It supports finite RDFS and an explicitly limited OWL-RL
core vocabulary. Unsupported constructs fail instead of silently claiming full
OWL support. The precise release profile and limitations are in the plugin guide.

## Generic host changes

Plugin SDK v3 adds default-denied `snapshot`, `publish`, and `derived_status`
methods to the existing context interface. Plugin invocation receives a scoped
host context tied to the registered plugin ID. Property-function contexts remain
read-only; ordinary plugins need not implement new methods. Compiled plugins must
be rebuilt for SDK v3; incompatible SDK manifests are rejected.

Snapshot exports a consistent iterator view of selected named/default graphs as
a merged N-Triples graph, retaining blank-node labels. The token hashes dataset
identity, selected graph names, and sorted quads. Publication accepts only a short
job name and resolves output/metadata IRIs in the invoking plugin's namespace.
It parses the complete result before mutation, checks the source token under the
shared dataset mutation lock, and commits graph replacement plus metadata in one
Oxigraph transaction. Supported Dataset/Database writes share this lock. Raw
Oxigraph mutation remains a trusted low-level escape hatch outside this contract.

The host contains no OWL rules, profile tables or sidecar logic. These generic
capabilities also support other materializers. The backend continues to expose
only generic plugin operations. CLI application composition opts into inference
via JSON configuration; the engine can also run without HTTP.

## Reasoning and publication

The plugin snapshots explicitly selected data/ontology graphs, calls a reasoner
adapter, checks its identity, and delegates publication to the host. Its operations
are `rebuild` and `status`. The default adapter uses the local sidecar; the public
`Reasoner` trait supports independently implemented adapters and declared profiles.

Each output graph contains deductions only, accompanied by generation metadata.
Full recomputation after modifications handles retraction without implementing
truth maintenance. Inference is explicit: ordinary SPARQL can query stale graphs,
so consumers must inspect status/rebuild. Transparent inference-aware querying,
incremental maintenance, derived-job dependency scheduling, per-fact proofs and
arbitrary custom rules are deferred.

OWL-RL reasoning is finite materialization under a declared supported subset. It
is not interchangeable with an OWL 2 DL entailment/classification service. Future
HermiT/ELK adapters must expose appropriately scoped contracts and validation.

## Validation and references

Tests cover atomic failure, changed-source rejection, blank-node joins, dataset
isolation, output integrity, real deductions, source deletions, persistence,
unsupported terms, reported inconsistency, and the real Studio/backend workflow.
These behavioral tests do not constitute full W3C conformance certification.

- [RDFLib OWL-RL](https://github.com/RDFLib/OWL-RL)
- [Deductive closure API and imports behavior](https://owl-rl.readthedocs.io/en/latest/owlrl.html)
- [W3C OWL 2 profiles](https://www.w3.org/TR/owl2-profiles/)
