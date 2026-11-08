# ADR 0012: Inference over outside data (virtual graph and federation inputs)

Status: accepted. Builds on ADR 0006 (derived graphs), 0010 (virtual graph and federation) and 0011 (composition).

## Problem

Inference jobs reasoned only over graphs stored in the dataset, so facts that live in Postgres (through Ontop) or in other
datasets had to be copied in before a reasoner or rule could use them, and the copy went stale unnoticed. Inference results
also could not be searched or joined back to the live sources.

## Decision

A job may declare `inputs`: CONSTRUCT or DESCRIBE queries run against a loopback sidecar when the job rebuilds.

```json
{ "name": "acme-live", "provider": "owlrl", "profile": "owlrl-core-v1",
  "source_graphs": ["urn:rdftool:examples:acme-ontology"],
  "inputs": [{ "name": "postgres", "kind": "sparql", "endpoint": "http://127.0.0.1:18090/sparql",
               "query": "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }" }] }
```

| Kind | Endpoint | Use |
| --- | --- | --- |
| `sparql` | a SPARQL Protocol endpoint such as Ontop | reason over live relational rows |
| `federation` | the Comunica sidecar (`source` optional) | rules over several sources at once, including other datasets |

- The fetched triples are merged with the local source snapshot and handed to the provider (OWL-RL, Jena, SPARQL rules
  alike). They are **never written to the store**; only the deductions are published, as before.
- Blank nodes from an input are renamed per input so they cannot clash with local ones. Inputs are limited to 8 per job and
  16 MiB each; the endpoint must be loopback, the query must be a CONSTRUCT or DESCRIBE, and `SERVICE` targets are still
  limited by the sidecar's allowlist. Ontop answers in Turtle, which is converted to N-Triples; the federation sidecar
  returns N-Triples directly.
- The derived graph's metadata records, per input, the query digest, triple count and content digest.
- **Staleness follows the outside data.** `status` re-runs each input and compares digests; a mismatch reports
  `state: "stale"` with `stale_inputs: [...]`, and an unreachable input reports `stale` with `input_error`. Local source
  changes are detected as before.

## What this enables

- `acme-live`: OWL-RL over the ontology plus live Postgres rows. It infers exactly the types it infers over the stored copy
  (checked in the end-to-end test), so the virtual graph can be reasoned over without materializing it.
- `portfolio-risk`: SPARQL rules over a federated CONSTRUCT that joins Postgres, the claims dataset and the finance dataset.
  Rules derive `rsk:atRisk`, a per-policy count of high-severity claims and a text `rsk:riskNote`.
- A hybrid semantic index over the derived graph (`risk-notes`, `graph` set to the derived graph) makes the inferred
  notes searchable, and a federated query joins each hit to live Postgres data. Inference, semantic search, the virtual
  graph and federation therefore feed each other: source data, derived facts and search sit in one query.

## Consequences and limits

- A rebuild depends on the sidecars being up; a stale-check does too (it fetches the inputs).
- Indexes over derived graphs go stale whenever the job is rebuilt; rebuild the index afterwards. Any new graph can also
  make an index that covers all graphs stale (the seed script rebuilds every index last).
- Outside data is not part of the store's source token, so the staleness check compares fetched content, not versions.
- Derived graphs can now be sources of other jobs (ADR 0015); before that, a job could only reach another job's output through a federation input.
- Not done: scheduled refresh, streaming large inputs, per-input credentials, and a provenance record linking each derived
  triple to the input rows that produced it (the spec in `docs/specs/inference-provenance.md` remains unimplemented).

## Verification

`scripts/test/federation_e2e.py`: jobs record their inputs; live-data inference equals stored-data inference; the rule
output for the portfolio is exact (Policy-1 has two high-severity claims, Policy-2 one); search over the inferred notes
returns the theft note first and joins it to Postgres; changing a Postgres row turns `acme-live` stale and names the
input, and rebuilding makes it ready again. Unit tests cover input validation, blank-node scoping, fetch, and failures.
