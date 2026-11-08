# Fact-level inference provenance

Status: proposed implementation specification, 2026-11-03. This is not a claim
that fact-level provenance has been implemented or validated.

[Confirmed intent](../intent/inference-provenance.md).

## Existing behavior and boundaries

`plugins/inference` selects providers and publishes deductions through
`rdf-plugin-api::ReadContext`. `rdf-engine/src/derived.rs` atomically replaces one
output graph and generation metadata. Its snapshot flattens selected source
quads to N-Triples. The metadata records source/configuration/provider identity
and output integrity, not generating rules or supporting statements.

Extend that inference plugin. Providers capture rule evidence; the plugin
validates and encodes it; the host publishes an opaque RDF artifact bundle.
The Axum backend exposes existing plugin operations and SPARQL. Studio can query
the resulting RDF through its existing query editor. No new explanation service,
SPARQL extension, reasoner implementation in the host, or UI is required.

## Provider capability and compatibility

Add a job option `provenance: "off" | "required"`, default `off` for compatibility.
Provider identity advertises supported proof profiles. Required mode must fail
before publication if a provider/profile cannot produce validated evidence.
Never silently fall back to a deduction-only result.

| Provider/profile | First implementation |
| --- | --- |
| Jena `jena-forward-v1` | Capture recorded rule derivations with logging enabled before preparation. |
| Native SPARQL, new `sparql-bgp-proof-v1` | Positive basic graph patterns and a variable/constant CONSTRUCT template; one evaluation yields both bindings and matched support triples. |
| Existing `sparql-construct-once-v1` | Remains available without proofs; reject required mode. |
| Current Python RDFS/OWL-RL profiles | Remain available without proofs; reject required mode until the provider records actual rule firings. |

Jena provides derivation logging and rule derivation objects with rule, conclusion,
and matches; extraction still needs validation against our pinned Jena version.
See [Jena inference documentation](https://jena.apache.org/documentation/inference/)
and [RuleDerivation API](https://jena.apache.org/documentation/javadoc/jena/org.apache.jena.core/org/apache/jena/reasoner/rulesys/RuleDerivation.html).
Do not claim enumeration of all possible or minimal proofs.

The restricted SPARQL profile rejects OPTIONAL, UNION, MINUS, NOT EXISTS,
aggregates, paths, subqueries, SERVICE, datasets, expressions, template blank
nodes, and unbound template variables. Use the parsed query algebra, not regular
expressions or reverse matching of output. All rules read the original merged
snapshot, preserving the existing once-only behavior. More expressive lineage
requires separate semantics for missing facts, aggregation and computed values.

## Snapshot and provider exchange

Version the SDK contract when introducing structured snapshots and publication
bundles. Preserve SDK v3 behavior through an explicit compatibility path; never
accept a different SDK version accidentally.

Extend snapshots with lossless source N-Quads from the *same* engine snapshot as
the merged N-Triples and token. Keep the source graph set and dataset identity in
the token. Preserve literal lexical forms, language tags, datatypes, and source
blank-node identity. Do not run independent queries to recover graph membership.

Add a versioned inference response containing:

- Deduction N-Triples and provider identity.
- Rule records: stable rule identifier, full rule text, profile and text digest.
- Fact records: structured RDF terms, classified as asserted or inferred.
- Derivation records: conclusion fact ID, rule ID, ordered supporting fact IDs.
- Proof profile/version and counts.

Validate all identifiers, term positions, rule references, support references and
conclusions before serialization. Every output deduction must have a finite
recorded witness reaching asserted facts. Reject fabricated, missing, dangling,
or ungrounded cyclic evidence. Cyclic alternative derivations may be retained
only when the fact also has a grounded witness. Track visited fact IDs during
extraction; do not recursively print traces and parse the printed output.

Jena's merged source model loses graph selection. Resolve each asserted support
against snapshot quads. If a triple occurs in multiple selected graphs, record
all occurrences as alternative locations of that same support, not as a claim
that the rule required all those graphs.

## RDF representation v1

Use RDF 1.1 reification and namespace `urn:rdftool:provenance:` (`proof:`).
No RDF-star dependency. A job has:

- Existing deductions graph `urn:rdftool:derived:inference:JOB`.
- Companion proof graph `urn:rdftool:derived:inference:JOB:provenance`.
- Existing metadata graph ending `:metadata`, extended with generation ID,
  proof profile, graph IRI, counts and integrity digests.

Generate a fresh opaque generation identifier on each successful rebuild. Scope
all fact, rule and derivation resource IRIs to it. Use collision-resistant
hashes of length-delimited RDF-term encodings for identity within a generation;
do not concatenate ambiguous strings or equate lexical variants of literals.
Blank-node labels are dataset-local identities, not portable global identifiers.
Retain input blank nodes across the bundle; reject provider-introduced blank
nodes in these initial proof profiles.

A fact is an `rdf:Statement` with `rdf:subject`, `rdf:predicate`, `rdf:object`, and
`proof:kind` (`proof:Asserted` or `proof:Inferred`). Asserted facts have one or more
`proof:sourceGraph` values; represent the default graph using the literal
`"default"` and named graphs using IRIs, so an actual IRI cannot collide with
the sentinel. Inferred facts link to each recorded `proof:derivation`.
A derivation has `proof:rule`, `proof:support` and `proof:conclusion`; each fact
also has `proof:dependsOn` links to the union of its recorded direct supports.
The latter enables traversal but does not preserve alternative-proof groupings.
Rules carry `proof:text`, `proof:digest` and `proof:profile`.
All records link to `proof:generation`; the generation carries snapshot token,
selected graphs, provider identity, configuration digest and capture profile.
Reification describes evidence; it does not itself assert the described triple.

Example inside the proof graph (illustrative IDs):

```turtle
@prefix proof: <urn:rdftool:provenance:> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix ex: <https://example.org/> .
<urn:g1:fact1> a rdf:Statement ;
  rdf:subject ex:alice ; rdf:predicate ex:worksInCountry ; rdf:object ex:France ;
  proof:kind proof:Inferred ; proof:generation <urn:g1> ;
  proof:derivation <urn:g1:d1> ; proof:dependsOn <urn:g1:f2>, <urn:g1:f3> .
<urn:g1:d1> proof:conclusion <urn:g1:fact1> ; proof:rule <urn:g1:r1> ;
  proof:generation <urn:g1> ; proof:support <urn:g1:f2>, <urn:g1:f3> .
<urn:g1:r1> proof:text "[worksIn: (?p <https://example.org/worksFor> ?c) (?c <https://example.org/locatedIn> ?country) -> (?p <https://example.org/worksInCountry> ?country)]" ;
  proof:profile "jena-forward-v1" .
<urn:g1:f2> a rdf:Statement ; proof:kind proof:Asserted ;
  rdf:subject ex:alice ; rdf:predicate ex:worksFor ; rdf:object ex:Acme ;
  proof:sourceGraph <urn:rules:data> ; proof:generation <urn:g1> .
<urn:g1:f3> a rdf:Statement ; proof:kind proof:Asserted ;
  rdf:subject ex:Acme ; rdf:predicate ex:locatedIn ; rdf:object ex:France ;
  proof:sourceGraph <urn:rules:data> ; proof:generation <urn:g1> .
```

## Ordinary SPARQL examples

Find the generating rule and its two supporting facts (expected Alice/worksFor/
Acme and Acme/locatedIn/France for the example above):

```sparql
PREFIX proof: <urn:rdftool:provenance:>
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX ex: <https://example.org/>
SELECT ?derivation ?rule ?text ?s ?p ?o ?sourceGraph WHERE {
  GRAPH <urn:rdftool:derived:inference:work-jena:provenance> {
    ?fact rdf:subject ex:alice ; rdf:predicate ex:worksInCountry ;
          rdf:object ex:France ; proof:derivation ?derivation .
    ?derivation proof:rule ?rule ; proof:support ?support .
    ?rule proof:text ?text .
    ?support rdf:subject ?s ; rdf:predicate ?p ; rdf:object ?o .
    OPTIONAL { ?support proof:sourceGraph ?sourceGraph }
  }
}
ORDER BY ?derivation ?s ?p ?o
```

Find asserted leaves across recorded chains (a union of evidence, not a single
minimal proof; retain derivation grouping with the first query when needed):

```sparql
PREFIX proof: <urn:rdftool:provenance:>
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX ex: <https://example.org/>
SELECT DISTINCT ?s ?p ?o ?sourceGraph WHERE {
  GRAPH <urn:rdftool:derived:inference:work-jena:provenance> {
    ?fact rdf:subject ex:alice ; rdf:predicate ex:worksOnContinent ;
          rdf:object ex:Europe ; proof:dependsOn+ ?leaf .
    ?leaf proof:kind proof:Asserted ; rdf:subject ?s ;
          rdf:predicate ?p ; rdf:object ?o ; proof:sourceGraph ?sourceGraph .
  }
}
ORDER BY ?s ?p ?o
```

A stale source does not erase the historical evidence in the last generation.
Check the inference plugin's status operation before treating an explanation as
current. SPARQL remains a direct read and does not automatically enforce freshness.

## Atomic publication and lifecycle

Introduce host-managed `publish_bundle` with fixed artifact roles, not arbitrary
filesystem paths or graph destinations. The host derives owned graph IRIs and
rejects collisions, unknown roles and cross-plugin destinations. Parse and
validate the complete bundle before mutation. Under the existing mutation gate,
recheck the source token, then replace deductions, proofs and metadata in one
Oxigraph transaction. A proof-free rebuild removes any old proof graph in that
same transaction. No mixed generations are observable.

Extend status checks to hash both deductions and proof artifacts. Include proof
mode/profile in configuration fingerprints. Edits to either artifact make status
stale; missing proof graph in required mode is invalid. No append-only history in
v1. Rebuild after source deletions; remove obsolete deductions and explanations.
Backup/restore carries all graphs, with existing dataset-identity freshness rules.
Do not use derived/proof graphs as inference inputs in this release.

Initial hard publication limits: existing 16 MiB deductions and source limits,
64 MiB proof RDF, 200,000 derivations, 1,000,000 support edges, and existing 64 KiB
metadata. Limits are counted before/while accumulating output; reject overflow
and preserve the old generation. Never silently truncate proofs. Provider-side
limits must stop extraction early; response-body and subprocess output reads
must be bounded too. Keep Jena subprocess timeout/container limits. Native SPARQL
cancellation remains cooperative and does not promise a hard execution deadline.

## Ordered implementation and acceptance

1. SDK snapshot/bundle contract and engine tests: graph identity, blank nodes,
   mixed-generation prevention, token race, malformed proof rollback, bounds,
   stale/tampered artifacts, proof-free rebuild, reopen and backup.
2. Provider capability negotiation, versioned response, validated RDF vocabulary,
   and job configuration. Old jobs work unchanged; required mode fails explicitly
   for unsupported profiles. Empty results publish an empty, valid generation.
3. Jena capture: joined rule, recursive chain, two independent derivations,
   duplicate source triples in two graphs, cyclic rules with a grounded path,
   language/datatype/blank-node fidelity, memory/output limit failure. Compare
   published supports against the actual rule matches, not another adapter path.
4. Restricted native SPARQL profile: matched bindings and CONSTRUCT output from
   one evaluation, repeated variables, constants, duplicate solutions, empty
   patterns, multiple template triples, invalid-profile rejection. Verify every
   witness by applying its recorded binding to the rule body and head.
5. End-to-end tests invoke rebuild through HTTP, query proofs with standard
   SPARQL, edit/delete source facts, observe stale status, rebuild, restart and
   re-query. Run real Jena sidecar tests separately from mocked-provider tests.
6. Publish runnable fixtures and exact expected query results. Existing UI query
   editor suffices; no claim of an explanation UI or complete OWL provenance.

Release gate: all supported deductions have grounded recorded evidence, required
mode never publishes unsupported/incomplete proofs, and failed rebuilds leave the
previous complete generation intact. Report other providers as unsupported for
fact proofs, without implying that their existing inference is unavailable.
