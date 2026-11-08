# ADR 0011: Composing plugins in one federated query

Status: accepted. Builds on ADR 0009 (hybrid search) and ADR 0010 (virtual graph and federation).

## Goal

Use the plugins together: a single SPARQL query that mixes semantic/hybrid search, inferred facts, live relational rows
(Ontop over Postgres), materialized graphs and other datasets. Each plugin stays independent; composition happens in the
query, not in plugin code.

## How it composes

Every dataset's `/query` endpoint is a SPARQL endpoint that already runs that dataset's plugins (property functions such
as `search:hybrid`, derived inference graphs). The federation plugin evaluates `SERVICE` clauses over endpoints on its
allowlist, so each plugin is reached through the endpoint that hosts it:

| Capability | Where it runs | How a federated query reaches it |
| --- | --- | --- |
| Semantic, keyword and hybrid search | the dataset that owns the index (`claims`, index `narratives`) | `SERVICE <.../d/claims/query> { SELECT ... { (?s ?p ?o ?score) search:hybrid (...) } }` |
| Inference | a derived graph in the dataset (`default`, job `acme`) | `SERVICE <.../query> { SELECT ... { GRAPH <urn:rdftool:derived:inference:acme> { ... } } }` |
| Materialized RDF | a named graph in a dataset | the same sub-select form |
| Virtual graph | Ontop over Postgres | `SERVICE <http://ontop:8080/sparql> { ... }` |
| Other business units | their own datasets | `SERVICE <.../d/finance/query> { ... }` |

Variables shared between `SERVICE` blocks are the joins; the policy IRI is the key across Postgres, the materialized ACME
graph, the inferred types and the finance dataset.

## Engine change: property calls survive forwarding

A federation engine forwards a sub-pattern with blank nodes rewritten as variables (`?g_0`). The property-function
rewriter only recognised list cells that were blank nodes, so `search:hybrid` failed with "wrong property arity" when sent
through Comunica. It now also accepts a variable as a list cell when that variable heads an `rdf:first` pattern in the
same basic graph pattern. This reads only the query's own patterns and consumes them, so stored RDF lists are still never
traversed, and a plain variable argument without an `rdf:first` pattern behaves as before.
(`crates/rdf-engine/src/properties.rs`, test `property_calls_survive_blank_nodes_rewritten_to_variables`.)

## Rules for writing composed queries

- Put a search call or a named-graph read inside `SERVICE <endpoint> { SELECT ... { ... } }`. A bare `GRAPH` block can
  silently return no rows (ADR 0010, known limit); a property call is safest in a sub-select too.
- Everything outside `SERVICE` runs on the selected dataset's default graph.
- Use inner joins between blocks. An `OPTIONAL` on a variable that may be unbound joins against every row of the next
  source (the correct SPARQL result, rarely the intended one).
- Hybrid scores are rank-based (reciprocal rank fusion), so filter by rank (`top-k`), not by a score threshold.
- All endpoints must be on the allowlist; composing does not widen what the sidecar may contact.

## Demo wiring

- `claims` dataset: claim narratives plus a hybrid index `narratives` (`deploy/local/tenants/claims/semantic.json`).
- `default` dataset: ACME ontology and instances materialized from the same R2RML as the Postgres source, and the
  OWL-RL job `acme` (`deploy/local/tenants/default/inference.json`) that derives, for example, that a claim's loss reserve
  is a `LossReserve` from the ontology's `rdfs:range`.
- `scripts/seed/federation_demo.py` builds both; Studio's "Composed" group has the examples.

## Verification

`scripts/test/federation_e2e.py` runs the composed queries against the live stack: materialized graph and Postgres agree;
Postgres rows receive inferred types; search, Postgres, finance and inference answer one query with the expected single row
for "burst pipe flooding"; keyword search returns exactly the two claims containing "garage" while hybrid also returns
meaning matches.

## Not done

- A planner that routes a plugin call to the dataset that owns it; today the query author names the endpoint.
- Result provenance: which source and which plugin produced each row.
- Pushing search `top-k` or selective bindings across sources (every endpoint is queried with Comunica's default strategy).
