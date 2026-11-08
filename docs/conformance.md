# Standards conformance: where we stand

Measured against the W3C test suites, pinned in `scripts/conformance.sh`:

- `rdf-tests` at `d01b141`;
- the OWL 2 RL test-case export, checked by its sha256.

Every case that does not pass is listed with its cause in `tests/conformance/*.json`. `just conformance` fails on any change in either direction:

- a regression;
- a fixed case still listed as failing.

`just conformance-update <suite> ...` rewrites a baseline on purpose.

## RDF syntaxes

991 of 993 cases pass.

| Status | Count | Cause |
|---|---|---|
| Passed | 991 | N-Triples, N-Quads, Turtle, TriG, RDF/XML |
| Failed | 2 | `rdf:XMLLiteral` values are not canonicalised (`xml-canon-test001/002`) |

## SPARQL 1.0 and 1.1 (1,156 cases)

The suite runs two ways:

- **Oxigraph:** the upstream engine directly.
- **Database:** our `rdf-engine` path in front of it, which adds the parser, the local-only policy, the property-function rewrite and the default plugins.

| Status | Oxigraph | Database |
|---|---|---|
| Passed | 952 | 949 |
| Equal only by value (`value-only`) | 22 | 22 |
| Failed | 44 | 47 |
| Not run (`unsupported`) | 138 | 138 |

**Our layer.** The two columns differ only by the three cases our policy refuses: `SERVICE` and `LOAD` in the local engine, since federation is a plugin. The property-function rewrite changes no result in the standard suite.

### Gaps that come from Oxigraph 0.5.13

| Cases | Cause | Effect |
|---|---|---|
| 36 (22 value-only, 14 failed) | **Lexical forms are canonicalised on storage.** `"01"^^xsd:integer` is stored as `1`, `"1.0e0"` as `"1"^^xsd:double`, and `+00:00` as `Z`. | Values are right, but literals do not round-trip byte for byte. Pattern matching treats `"01"` and `"1"` as the same term, as do `sameTerm` and `DISTINCT`. |
| 18 | **Tests added to the W3C suite in 2025–26** (`zero_or_*_set`, `graph-minus`, `VALUES` inside `GRAPH`, `FROM NAMED` scoping, `subquery15/16`, …) | Newer clarifications that this Oxigraph version predates. Re-run after an upgrade. |
| 2 | `GROUP_CONCAT` keeps a shared language tag | SPARQL 1.1 requires a simple literal. |
| 2 | Parser: `TRUE`/`FALSE` in upper case is rejected; `syn-bad-26` is accepted | Edge cases. |
| 2 | Untriaged: `date-2` and `bnode01` | |

**Not run:**
- 68 HTTP protocol, graph store protocol and service description cases. These need an HTTP runner against `rdftool serve`.
- 70 entailment-regime cases, which are covered in the next section.
- 6 `SERVICE` cases that need remote endpoints.

## Inference

Each run goes through the same HTTP adapters that inference jobs use.

| Suite | owlrl sidecar | Jena sidecar with standard RDFS rules |
|---|---|---|
| RDF 1.1 Semantics, `rdf-mt` (48) | 14 pass, 10 fail, 24 not run | 14 pass, 10 fail, 24 not run |
| SPARQL 1.1 entailment regimes (70) | 6 pass, 64 not run | 32 pass, 4 fail, 34 not run |
| OWL 2 RL-profile test cases (126 checks over 91 tests) | 39 pass, 10 fail, 77 not run | no OWL profile |

### Why cases fail inside the profiles we run

- **No axiomatic triples.** Both reasoners are configured without them (`axiomatic_triples=False`). Two kinds of conclusion are missed:
  - `rdf:_1 a rdfs:ContainerMembershipProperty`;
  - reflexive `rdfs:subClassOf` for classes known only through `rdfs:range rdfs:Class`.
- **No datatype reasoning** (D-entailment). Ill-typed literal clashes are not detected. `xsd:short ⊑ xsd:integer`-style range conclusions are not drawn.
- **OWL 2 RL rules derive no `owl:differentFrom`.** That needs proof by contradiction.
- **OWL Full structural conclusions are not drawn.** Examples: "a list exists", "this restriction class exists".

### Why cases are not run

- **`owlrl-core-v1` is a deliberate subset of OWL RL.** It rejects 104 cases outright. The terms involved, in order of how many cases they block, are:
  - `intersectionOf`, `propertyChainAxiom`;
  - annotations;
  - `hasKey`, `AllDisjointClasses`;
  - `complementOf`, reflexive properties;
  - `NegativePropertyAssertion`, `AllDisjointProperties`;
  - restrictions, `oneOf`, `AllDifferent`.
- **`rdfs-v1` rejects any data that mentions an OWL term.** RDFS treats such terms as plain IRIs. Because of this, 38 RDFS-regime SPARQL cases run on Jena but not on owlrl.
- **Regimes with no matching profile:**
  - RDF-only and simple entailment;
  - OWL Direct Semantics;
  - RIF;
  - D-entailment.
- **OWL tests we cannot read:** tests given only in functional syntax (5), and tests that use `owl:imports` (2).

For OWL in depth, see [owl-coverage.md](owl-coverage.md). It covers what lifting our profile restriction gains (measured), the soundness and cost checks, and a staged roadmap.

## Changes that would close the most gaps

1. **Widen the profiles.**
   - Let `rdfs-v1` accept OWL terms.
   - Enable axiomatic triples, scoped to the vocabulary in use.
   - Each OWL construct added to `owlrl-core-v1` turns a block of not-run cases into measured ones.
2. **Add an HTTP conformance runner** for the 68 protocol cases. It would run against `rdftool serve` and the gateway, so it tests our server rather than Oxigraph.
3. **Upgrade Oxigraph** to pick up the 2025–26 clarifications. Then decide whether lexical-form canonicalisation is acceptable (provenance and round-tripping) or needs work upstream.
