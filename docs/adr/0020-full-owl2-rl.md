# ADR 0020: Full OWL 2 RL — profiles, clashes as results, and a native engine

Status: accepted. Steps 1–4 of `docs/owl-coverage.md`.

## Context

The OWL-RL sidecar's `owlrl-core-v1` refused most OWL. It passed 39 of 126 OWL 2 RL test checks
and refused 77 before reasoning. An inconsistent input failed the job with an error string. OWL
jobs could not explain a deduction, and closure ran in Python, about 3,000 input triples per
second.

## Decision

### New profiles, never changed profiles

The OWL-RL sidecar adds two profiles:

- `rdfs-v2`: OWL terms are plain IRIs.
- `owlrl-full-v1`: all of OWL 2 RL as owlrl implements it.

`rdfs-v1` and `owlrl-core-v1` are unchanged. A profile and the reasoner's identity are part of
every job's fingerprint, so a changed profile would silently change what existing jobs mean.

### Scoped axioms

The v2 profiles add axiomatic triples, filtered:

- Nothing is said about an RDF, RDFS, OWL or XSD term the input never mentions.
- Nothing is said that holds of every resource (`x a rdfs:Resource`, `x a owl:Thing`).

Reflexive `owl:sameAs` is kept, because answers that walk `sameAs` chains depend on it. The
conformance baseline caught two regressions when it was dropped.

### Clashes are results

The sidecar uses protocol 2. An inconsistent input answers 200 with `clashes`. Providers raise the
typed error `rdf_plugin_inference::Inconsistent`.

- `rebuild` returns `state: inconsistent` with the clashes and leaves the derived graph untouched.
- `status` reports `inconsistent` while the job's source token, outside inputs and configuration
  are still the ones that clashed. The record is kept in memory per process.
- A cascade marks jobs that read an inconsistent job as `blocked`.
- A rule test can expect `inconsistent: true`.

### A native OWL 2 RL engine

Provider kind `owlrl-native`, profile `owl2rl-v1`, in `rdf-inference-providers`.

- **Rules:** the OWL 2 RL/RDF tables for equality, properties, classes, class axioms and schema
  (4, 5, 6, 7 and 9), evaluated naively to a fixpoint over indexed, interned triples. The
  datatype rules (table 8) are not implemented.
- **Justifications:** the first justification of every derived triple is kept, as the rule and
  the triples it matched. `explain` returns the proof. A premise that the same job deduced first
  is nested (`triple`, `rule`, `premises`), and the plugin traces each leaf to its graph or
  outside input.
- **Clashes:** a clash carries its rule and premises.
- **Data values:** two different values of the same datatype for a functional or max-1 property
  are a clash. owlrl misses this (`Keys-006`).
- **Output policy:** the same as `owlrl-full-v1`.

## Results

| Reasoner and profile | OWL 2 RL tests (126) | SPARQL entailment (70) | RDF 1.1 Semantics (48) | Total pass |
|---|---|---|---|---|
| owlrl `owlrl-core-v1` / `rdfs-v1` | 39 | 6 | 14 | 59 |
| owlrl `owlrl-full-v1` / `rdfs-v2` | 97 | 41 | 17 | 155 |
| native `owl2rl-v1` (no RDFS profile) | **98** | 36 | not offered | 134 |

On OWL conclusions the native engine and owlrl agree case for case, except `Keys-006`, which only
the native engine detects.

On ACME, every native deduction is also an owlrl one: 254 of 254. owlrl adds 139 triples from
its RDFS layer and axioms (`p a rdf:Property`, `p rdfs:domain rdfs:Resource`, `C a rdfs:Class`),
which OWL 2 RL/RDF does not contain.

Cost, on a 60,014-triple synthetic ontology with intersections, a property chain and a
restriction:

| Engine | Time |
|---|---|
| Native (release build) | 0.16 s |
| owlrl `owlrl-full-v1` | 42.8 s |

The owlrl figure is close to the sidecar's 60 s timeout, so the native engine is the choice for
large inputs.

## Consequences

- A sidecar upgrade to protocol 2 changes its identity, so every owlrl job reads as stale once
  and needs one rebuild. Results for the v1 profiles are unchanged.
- The native engine does not do datatype reasoning, so 4 of the W3C cases need it. It offers
  no RDFS-only profile.
- Its explanations name one justification per triple, the first it found. Other valid ones
  exist.

## Later (same day)

- `p rdfs:domain owl:Thing` and `rdfs:range owl:Thing` (or `rdfs:Resource`) count as true of
  everything too. Both the v2 profiles and the native engine now leave them out, and their
  identities carry `output_policy: 2`, so jobs built earlier read as stale. On ACME this gives
  216 triples for the native engine and 313 for `owlrl-full-v1`. Conformance is unchanged.
- The native engine is now the default for the bundled OWL jobs (`vehicles`, `acme`,
  `acme-live`). `acme-full` stays on owlrl as the reference implementation.
- Clash records persist across restarts. They are kept in the job's metadata graph through
  `ReadContext::record_attempt`, and publishing clears them.
- The native engine evaluates its hottest rules semi-naively: equality copying, domain/range,
  subproperty, transitivity, subclass, and subclass/subproperty transitivity only join triples
  new in the last round. Premise lists are built only for conclusions that are actually new. On
  six inputs, the old and new engines produced byte-identical closures. Times in release build:

  | Input | Before | After |
  |---|---|---|
  | 500-node transitive chain (125k derived) | 1.54 s | 0.57 s |
  | Deep hierarchy (144k derived) | 0.30 s | 0.19 s |

  The crate also compiles optimised in the dev profile the local stack runs. There, the chain
  went from 12 s to 0.66 s.

