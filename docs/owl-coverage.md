# OWL reasoning: coverage, gaps, and a path to useful

Measured on 2026-11-08 against three suites:

- the W3C OWL 2 RL-profile test cases (91 tests, 126 checks);
- the RDF 1.1 Semantics tests (`rdf-mt`);
- the SPARQL 1.1 entailment-regime tests.

All runs go through the product's own reasoner adapters. How the suites run, and the baselines that hold them, are described in [conformance.md](conformance.md) and ADR 0018.

## Status, 2026-11-08: steps 1–4 implemented (ADR 0020)

| Step | Done |
|---|---|
| 1. Full OWL 2 RL | Profiles `owlrl-full-v1` and `rdfs-v2`. OWL 2 RL tests 39 → 97 passing, all entailment suites 59 → 155. |
| 2. Inconsistency as a result | Clashes reported with the terms involved, the last good output kept, cascades blocked, and rule tests can expect a clash. |
| 3. Scoped axioms | Part of the v2 profiles, which add +3 RDF Semantics and +4 SPARQL entailment cases, with reflexive `sameAs` kept. |
| 4. Native engine | `owlrl-native`. 98 OWL 2 RL checks (one more than owlrl: `Keys-006`), about 270× faster on 60k triples, and explanations with nested proofs. Clashes name their rule and premises. |

Still open:

- step 5, a DL reasoner (only if class-level questions are needed);
- datatype reasoning;
- `differentFrom` by contradiction and structural conclusions, which are beyond OWL 2 RL.

The rest of this document is the analysis as it was before the work.

## Summary

- **Our profile restriction is the main gap.**
  - **Today:** we pass 39 of 126 OWL 2 RL checks. 77 are refused before any reasoning happens, because `owlrl-core-v1` allows only a short list of OWL terms.
  - **With the restriction removed:** the same reasoner passes 98. Nothing becomes unsound, and in-profile results do not change.
- **The remaining misses are mostly inherent to OWL 2 RL**, not bugs. Rule-based RL reasoning does not:
  - derive `owl:differentFrom`;
  - build complement or restriction classes;
  - reason about datatypes.

  Closing those needs a different kind of reasoner (a DL reasoner), not a fix.
- **Cost is not the obstacle.** Full OWL 2 RL closure in the sidecar runs at about 3,000 input triples per second. That is roughly 35 s at our 100,000-triple cap, inside the 60 s limit.

## What we measured

### Today (deployed sidecars)

| Suite | Pass | Fail | Not run | Why not run |
|---|---|---|---|---|
| OWL 2 RL tests (126) | 39 | 10 | 77 | 66 use OWL terms outside `owlrl-core-v1`. 11 use Direct Semantics only, functional syntax, or `owl:imports`. |
| SPARQL entailment regimes (70) | 6 | 0 | 64 | 38 rejected by `rdfs-v1`, which refuses any OWL term. 26 use regimes we do not offer: OWL Direct, RIF, D, RDF-only. |
| RDF 1.1 Semantics (48) | 14 | 10 | 24 | RDF-only and simple regimes have no profile. |

Breakdown of the 126 OWL checks:

| Check | Pass | Fail | Not run |
|---|---|---|---|
| Consistent ontology accepted | 27 | 0 | 49 |
| Inconsistency detected | 3 | 0 | 11 |
| Non-entailment respected (soundness) | 3 | 0 | 3 |
| Entailment derived (completeness) | 6 | 10 | 14 |

Terms that cause rejection, by number of checks blocked:

| Term | Checks blocked |
|---|---|
| `intersectionOf` | 11 |
| `propertyChainAxiom` | 9 |
| `AnnotationProperty` | 7 |
| `hasKey` | 6 |
| Annotation axioms (`Axiom`, `annotatedSource` and similar) | 11 |
| `members`, `AllDisjointClasses`, `AllDisjointProperties` | 9 |
| `complementOf` | 2 |
| `ReflexiveProperty` | 2 |
| `NegativePropertyAssertion` | 2 |
| Restrictions, `oneOf`, `AllDifferent`, asymmetric and irreflexive properties | 1–2 each |

### Experiment: the same reasoner without the profile restriction

I started temporary copies of the owlrl sidecar image in two variants:

- **No restriction:** the term allowlist and the "class expressions must be named" check removed.
- **No restriction, plus axioms:** also with owlrl's axiomatic and datatype triples enabled.

The deployed sidecar was not changed.

| Suite | Deployed | No restriction | No restriction, plus axioms |
|---|---|---|---|
| OWL 2 RL tests: pass / fail / not run | 39 / 10 / 77 | **98** / 17 / 11 | **99** / 16 / 11 |
| SPARQL entailment: pass / fail / not run | 6 / 0 / 64 | **38** / 6 / 26 | **42** / 2 / 26 |
| RDF 1.1 Semantics: pass / fail / not run | 14 / 10 / 24 | 14 / 10 / 24 | **17** / 7 / 24 |

**Soundness held.**

- All 72 consistency checks passed, so no ontology was wrongly reported inconsistent.
- All 6 non-entailment checks passed.

Every remaining failure is something not derived. Nothing was derived wrongly.

**In-profile results are unchanged.** On the ACME insurance graph (392 triples), both variants derive the same 409 triples in 0.19 s.

**Cost.** I used a synthetic ontology with intersections, a property chain, a `someValuesFrom` restriction and a cyclic `parent` chain over n individuals:

| Input triples | Derived | Time |
|---|---|---|
| 3,014 | 5,127 | 1.0 s |
| 15,014 | 25,127 | 4.8 s |
| 60,014 | 100,127 | 19.8 s |

Time grows roughly linearly with input on this ontology. Rule-heavy ontologies with many `sameAs` merges can be worse, and the 60 s worker timeout remains the guard.

## What still fails without the restriction, and why

| Failure | Checks | Fixable within OWL 2 RL? |
|---|---|---|
| `owl:differentFrom` conclusions, for example from functional, inverse-functional or disjoint properties | 4 | **No.** These need proof by contradiction ("if they were the same, the ontology would be inconsistent"). RL rules only use `differentFrom` to detect clashes. |
| Conclusions that a class expression *exists* (`complementOf` classes, restrictions, lists, disjointness written as an axiom node) | 7 | **No.** OWL Full structural reasoning, outside any rule profile. |
| `ReflexiveProperty` | 1 | **No.** Not part of OWL 2 RL. |
| Property chain implies `TransitiveProperty` (a schema-level conclusion) | 1 | **No.** RL rules derive facts, not new axioms of this kind. |
| Datatype ranges (`xsd:short` / `xsd:unsignedShort`) | 3 | Partly. owlrl's datatype axioms fixed 1 of the 3; the rest need real datatype reasoning. |
| `hasKey` inconsistency not detected (`Keys-006`) | 1 | **Possibly an owlrl bug** in rule `prp-key`. Worth a minimal reproduction upstream. |
| RDF 1.1 Semantics: ill-typed literal clashes | 7 | Not with owlrl, even with its datatype axioms on. Needs literal validation. |
| SPARQL entailment `sparqldl-11/12` | 2 | Expected answers follow Direct-Semantics conventions (which classes count as answers). A regime-level limitation, not one of reasoning. |

## Getting to a useful stage

### Stage 1: offer full OWL 2 RL

- **What changes:**
  - Add a new profile, `owlrl-full-v1`, with no term allowlist. Keep the existing size and time limits.
  - Add `rdfs-v2`, which treats OWL terms as plain IRIs instead of rejecting them.
  - Keep `owlrl-core-v1` and `rdfs-v1` unchanged. The profile name and the reasoner's identity are part of every job's fingerprint, so changing a profile in place would make existing jobs stale and change their meaning silently.
- **Gain:**
  - OWL 2 RL checks: 39 → 98.
  - SPARQL entailment: 6 → 38.
  - Unblocked constructs ontologies actually use: intersections, property chains, keys, `AllDisjoint*`, annotations, restrictions.
- **Work:**
  - The sidecar profile switch.
  - The inference job docs.
  - A test per newly admitted construct.
  - A re-run of `just conformance-update entailment --reasoner owlrl`.

  The conformance suite already measures the result.

### Stage 2: make inconsistency a result, not a failure

- **Today:** an inconsistent input fails the whole job with an error string.
- **Proposal:** record which axiom clashed and on which individuals, for example "x is both A and B, which are disjoint". Publish it as job status, and keep the last good output.
- **Why it matters:** with keys and disjointness admitted, this becomes the most common way the reasoner "says something".
- **Link to explanations:** it fits the existing explanation feature (ADR 0017). A clash is a deduction of `owl:Nothing` with premises.

### Stage 3: axiomatic triples, scoped

- **Gain:** owlrl's axiomatic option fixes 3 RDF-Semantics cases and 4 SPARQL-entailment cases. These are reflexive `subClassOf` and the `rdf:_n` container rules.
- **Caveat:** it adds the axioms for the whole RDF/RDFS vocabulary to every job's output.
- **Proposal:** a profile flag that adds axioms only for vocabulary terms that appear in the input. That gives the same conclusions without the noise.

### Stage 4: explanations for OWL deductions

- **Today:** OWL-RL and Jena jobs answer `explain` with `supported: false`.
- **Option:** implement the OWL 2 RL rule table as rules in our own engine. The SPARQL-rules provider would need a fixpoint loop; today it runs one pass. Explanations would then work the way they already do for SPARQL rules.
- **Approach:** treat this as a second implementation, and keep owlrl as the reference. The conformance suite compares them.
- **Payoff:** explanations, and no Python process per job.

### Stage 5: beyond OWL 2 RL (only if needed)

The failures RL cannot close all need a Direct-Semantics (DL) reasoner. These are `differentFrom` by contradiction, class-expression conclusions, and classification of the class hierarchy itself. The options:

- **EL profile, for example ELK:** fast classification of large class hierarchies. It can run in the existing JVM sidecar alongside Jena.
- **Full DL, for example HermiT or Openllet:** complete for OWL 2 DL, with exponential worst cases. Suited to validating and classifying ontologies (TBox), not to bulk data.
- **QL:** query-time reasoning through Ontop (ADR 0016). It is already in place for the virtual graph.

The suite's 4 Direct-Semantics-only tests and the 18 OWL-Direct SPARQL-entailment tests would measure this stage. Do it when a user needs class-level answers ("which classes are unsatisfiable?", "what does this class subsume?"), not for instance data.

### Datatype checking

Ill-typed literal clashes are better caught by validation than by entailment. For example, `"abc"^^xsd:integer`, or a value outside a property's datatype range. SHACL (`sh:datatype`), which another session is currently adding, is the practical tool. Stage 1 does not depend on it.

## Suggested order

1. **Stage 1.** Large gain, low risk, and soundness already checked.
2. **Stage 2.** Makes the newly admitted constructs usable.
3. **Stage 3** if RDFS completeness matters to users. Otherwise skip it.
4. **Stage 4** if explanations for OWL jobs matter. It also removes the per-job Python process.
5. **Stage 5** on demand.

## Reproducing

- The temporary variants are the deployed image started with the worker's allowlist checks replaced by `pass`. The second variant also sets `axiomatic_triples=True, datatype_axioms=True`.
- Each variant runs on its own loopback port.
- Each variant is measured with `rdftool test-suite --manifest <suite> --reasoner owlrl=http://127.0.0.1:<port>`.
- The deployed sidecar was not modified, and the containers were removed afterwards.
