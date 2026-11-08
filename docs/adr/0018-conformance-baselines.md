# ADR 0018: W3C conformance held to committed baselines

Status: accepted.

## Context

Conformance used to run only the W3C syntax suites, and it ran them through Oxigraph directly. As a result:

- A pass said nothing about our own query path: the parser, the policy and the property-function rewrite.
- Nothing measured reasoning.
- The SPARQL evaluation manifests were never run. Run by hand, about 1 in 6 cases "failed", mostly because of harness bugs:
  - file and published IRIs were mixed;
  - the result formats were not read;
  - numbers were compared lexically.

## Decision

- **One runner, three dimensions.** `rdftool test-suite` takes `--engine oxigraph|database` and `--reasoner owlrl=<url>|jena-rdfs=<url>`.
  - `database` runs each case through `rdf-engine` with the default plugins.
  - A reasoner is reached through the same HTTP adapters inference jobs use.
- **Files are addressed by their published IRIs.** Inside a `rdf-tests` checkout, every file is addressed by its published IRI. Manifests, data, graph names and query bases then agree with the expected results.
- **Honest statuses.**
  - `unsupported` names the reason: an HTTP runner is needed, a regime is not offered, or the input is outside the reasoner's profile.
  - `value-only` marks results that match only when numbers are compared by value. That is, the engine changed a lexical form. These cases are not counted as passed.
- **Baselines.** `tests/conformance/<suite>-<variant>.json` lists each non-passing case with a cause. `scripts/conformance/baseline.py` fails both:
  - on a regression;
  - on a listed case that now passes.

  So the list shrinks only on purpose. Suites covered: RDF syntaxes, SPARQL 1.0 + 1.1, and entailment (RDF 1.1 Semantics, SPARQL entailment regimes, OWL 2 RL test cases).
- **CI.** The weekly extended-validation workflow starts the reasoner sidecars and runs everything. Locally, run `just conformance`.

## Consequences

- `docs/conformance.md` is the readable gap report. Its numbers come from the baselines.
- Upgrading Oxigraph or widening a reasoner profile shows up as cases that now pass. Those cases must be removed from the baseline explicitly, with `just conformance-update`.
- The OWL 2 RL suite's pass rate is meaningful only inside the declared profile. Every rejected case is listed as `outside-declared-profile`, together with the rejected term.
