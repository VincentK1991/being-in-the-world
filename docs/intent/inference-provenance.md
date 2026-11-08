# Inference provenance: confirmed intent

Confirmed during the provenance interview; recorded 2026-11-03.

- Outcome: explain an inferred fact by retrieving the rule and supporting facts
  that produced it.
- User: a developer inspecting and validating this RDF store's inference results.
- Why now: inference produces deductions, but does not retain fact-level evidence.
- Success: ordinary SPARQL can retrieve a deduction's generating rule and supports,
  including intermediate inferred facts.
- Constraint: extend the existing inference plugin and provider contracts. Store
  explanations as ordinary RDF; keep the engine independent of reasoner details.
- Out of scope: new SPARQL syntax, a separate explanation plugin, whole-query-result
  lineage, query-plan explanations, and dedicated explanation UI in the first version.

The additional work is capturing and storing evidence when rules execute.
SPARQL already provides the language for reading that evidence once it exists.

See the [implementation specification](../specs/inference-provenance.md).
