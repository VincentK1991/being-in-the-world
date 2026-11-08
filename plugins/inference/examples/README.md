# Executed inference examples

Executed on 2026-11-02 with the real pinned OWL-RL sidecar.
Input: [example.trig](../example.trig); source graphs are explicitly selected.

## classes

```sparql
SELECT ?type WHERE { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/car1> a ?type } FILTER(?type IN (<https://example.org/Vehicle>, <https://example.org/Asset>)) } ORDER BY ?type
```

```json
{
  "head": {
    "vars": [
      "type"
    ]
  },
  "results": {
    "bindings": [
      {
        "type": {
          "type": "uri",
          "value": "https://example.org/Asset"
        }
      },
      {
        "type": {
          "type": "uri",
          "value": "https://example.org/Vehicle"
        }
      }
    ]
  }
}
```

## inverse

```sparql
SELECT ?parent ?child WHERE { GRAPH <urn:rdftool:derived:inference:vehicles> { ?parent <https://example.org/hasChild> ?child } }
```

```json
{
  "head": {
    "vars": [
      "parent",
      "child"
    ]
  },
  "results": {
    "bindings": [
      {
        "parent": {
          "type": "uri",
          "value": "https://example.org/bob"
        },
        "child": {
          "type": "uri",
          "value": "https://example.org/alice"
        }
      }
    ]
  }
}
```

## transitive

```sparql
ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/office> <https://example.org/locatedIn> <https://example.org/country> } }
```

```json
{
  "head": {},
  "boolean": true
}
```

## isolation

```sparql
ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/secret> a <https://example.org/Vehicle> } }
```

```json
{
  "head": {},
  "boolean": false
}
```

The persistent demo is retained in `target/inference-demo/db`. Run `python3 plugins/inference/examples/run.py --output target/another-inference-demo` to reproduce in a new directory.
