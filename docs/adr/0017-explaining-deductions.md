# ADR 0017: Explaining deductions

Status: accepted. A first, partial implementation of `docs/specs/inference-provenance.md`.

## Decision

- `Reasoner::explain(profile, ntriples, rules, target)` returns derivations or `None` when a reasoner records no
  justifications (the default). The SPARQL rule provider implements it without changing how rules run: it matches each
  CONSTRUCT template triple against the target, fixes those variables, solves the rule's WHERE pattern with them fixed
  (an inline `VALUES` joined to the pattern), and instantiates the pattern's triple patterns under each solution to get the
  premises that exist in the data. For an aggregate it enumerates the group's members and says so.
- The inference plugin's `explain` operation finds the jobs whose derived graph holds the triple, asks the reasoner,
  and traces each premise: asserted in one of the job's source graphs, taken from an outside input, or deduced by an
  earlier job, which is explained recursively up to `depth`.
- Explanations are computed on request over current data, not recorded at rebuild time.

## Consequences

- No storage and no change to rebuild cost or published graphs; a stale job is flagged, and its explanation reflects the data now.
- Explanations are only as good as the reasoner: OWL-RL and Jena jobs are reported as unsupported. Getting justifications
  from them needs sidecar changes (or running OWL-RL rules in Rust) and is the main follow-up.
- Property paths, `GRAPH` blocks and `VALUES` inside a rule are marked incomplete; blank nodes are not traced.
- Each call is bounded (10 solutions per rule, 50 premises per derivation, 20 derivations, 400 lookups).
- Per-fact provenance recorded at publication time (spec) remains an option if on-demand cost or staleness matters.
