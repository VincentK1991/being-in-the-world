# Semantic search: executed SPARQL patterns

These outputs were produced on 2026-11-02 against the [catalog](catalog.ttl), real
Qdrant and local all-MiniLM-L6-v2 on the RTX 4090. They are not OpenAI results.
Scores below are rounded for readability; [raw results](results/) retain RDF term
types and full returned precision. Small CPU/GPU numeric differences are expected.

Reproduce from the repository root:

```sh
cargo build -p rdf-cli
python3 plugins/semantic-search/examples/run.py --output target/my-semantic-demo
```

The runner creates a persistent RDF database, builds the local `descriptions`
index, and executes queries 01–08. It refuses an existing demo database. The
configuration also declares an OpenAI index, but the local run never invokes it
or needs its credentials. Add `--openai` with `OPENAI_API_KEY` configured to build
both indexes and execute query 09 as well.

Each query is independently executable:

```sh
target/debug/rdftool --semantic-config plugins/semantic-search/example-providers.json \
  query --source target/my-semantic-demo/db \
  --query-file plugins/semantic-search/examples/queries/02-join-filter.rq
```

SPARQL property inputs are constant `(index, text, top-k)` values. These calls
can join ordinary RDF patterns; correlated variable inputs and calls inside
GRAPH/FROM/UPDATE remain outside the supported property-function profile.
An index can itself be restricted to a named graph through its configuration.

## 01-ranked-hits

One result per matching quad: the car appears twice because it has two matching descriptions.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?subject ?object ?score WHERE {
  (?subject ?predicate ?object ?score)
    search:matches ("descriptions" "battery powered vehicles" 4) .
} ORDER BY DESC(?score)
```

| subject | object | score |
| --- | --- | --- |
| ex:car | An electric automobile stores power in rechargeable batteries. | 0.6886 |
| ex:car | Electric motors propel this road vehicle. | 0.5558 |
| ex:bus | A battery powered city bus carries passengers on public transit routes. | 0.4396 |
| ex:bike | A pedal powered bicycle for commuting around the city without fuel. | 0.3320 |

## 02-join-filter

Distinct subjects are retrieved first, then ordinary RDF type and price filters are applied. Filters do not refill the top-k candidate set.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?price ?score WHERE {
  (?subject ?score) search:subjects ("descriptions" "clean city transportation" 6) .
  ?subject a ex:Vehicle ; rdfs:label ?label ; ex:price ?price .
  FILTER(?price < 50000)
} ORDER BY DESC(?score)
```

| label | price | score |
| --- | --- | --- |
| Commuter bicycle | 800 | 0.3815 |
| Electric car | 32000 | 0.2741 |

## 03-literal-objects

Returns original literal objects. The second hit is weak: top-k asks for candidates, not a guaranteed relevance cutoff. Use a score filter when appropriate.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?text ?score WHERE {
  (?text ?score) search:objects ("descriptions" "baking homemade bread" 2) .
} ORDER BY DESC(?score)
```

| text | score |
| --- | --- |
| A recipe for baking bread with flour and yeast. | 0.7663 |
| A pedal powered bicycle for commuting around the city without fuel. | 0.1370 |

## 04-optional

OPTIONAL preserves hits with no price. A missing value is unbound, not zero or an empty literal.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?price ?score WHERE {
  (?subject ?score) search:subjects ("descriptions" "warm food for dinner" 3) .
  ?subject rdfs:label ?label .
  OPTIONAL { ?subject ex:price ?price }
} ORDER BY DESC(?score)
```

| label | price | score |
| --- | --- | --- |
| Vegetable soup | unbound | 0.4401 |
| Bread recipe | unbound | 0.2316 |
| Galaxies | unbound | 0.0942 |

## 05-aggregate

Semantic subjects are deduplicated before aggregation; the two car descriptions do not count as two vehicles.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?category (COUNT(?subject) AS ?hits) (MAX(?score) AS ?bestScore) WHERE {
  (?subject ?score) search:subjects ("descriptions" "battery powered vehicles" 6) .
  ?subject ex:category ?category .
  FILTER(?score > 0.2)
} GROUP BY ?category ORDER BY DESC(?bestScore)
```

| category | hits | bestScore |
| --- | --- | --- |
| transport | 3 | 0.6886 |

## 06-union

Two independent semantic searches are combined with UNION, then joined to RDF labels.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?intent ?label ?score WHERE {
  { (?subject ?score) search:subjects ("descriptions" "battery powered vehicles" 1) . BIND("travel" AS ?intent) }
  UNION
  { (?subject ?score) search:subjects ("descriptions" "baking homemade bread" 1) . BIND("cooking" AS ?intent) }
  ?subject rdfs:label ?label .
} ORDER BY ?intent
```

| intent | label | score |
| --- | --- | --- |
| cooking | Bread recipe | 0.7663 |
| travel | Electric car | 0.6886 |

## 07-ask

ASK returns a boolean after combining semantic score and RDF type constraints.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
ASK {
  (?subject ?score) search:subjects ("descriptions" "battery powered vehicles" 3) .
  ?subject a ex:Vehicle . FILTER(?score > 0.5)
}
```

Result: **true**.

## 08-construct

CONSTRUCT returns an RDF graph describing the selected match; it does not write those triples into the database.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
CONSTRUCT { ?subject ex:semanticScore ?score ; rdfs:label ?label } WHERE {
  (?subject ?score) search:subjects ("descriptions" "baking homemade bread" 1) .
  ?subject rdfs:label ?label .
}
```

```turtle
<https://example.org/bread> <http://www.w3.org/2000/01/rdf-schema#label> "Bread recipe" ;
	<https://example.org/semanticScore> "0.76633406"^^<http://www.w3.org/2001/XMLSchema#double> .
```

## 09: Two embedding providers in one query

This query is executable after building both configured indexes with `--openai`.
It was **not run against OpenAI** in this session because no API key was available.
A separate integration test verified this mixed-index path with real local
embeddings and an explicitly mocked hosted endpoint. Its output is not presented
as hosted model relevance evidence.

```sparql
PREFIX search: <urn:rdftool:search:>
PREFIX ex: <https://example.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?provider ?subject ?score WHERE {
  { (?subject ?score) search:subjects ("descriptions" "battery powered vehicles" 2) . BIND("local" AS ?provider) }
  UNION
  { (?subject ?score) search:subjects ("descriptions_openai" "battery powered vehicles" 2) . BIND("openai" AS ?provider) }
} ORDER BY ?provider DESC(?score)
```

Results include a provider label and separate rankings. Do not interpret scores
from different embedding models as a single calibrated scale.
