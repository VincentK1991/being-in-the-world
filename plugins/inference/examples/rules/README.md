# Executed rule-provider examples

These results were obtained on 2026-11-02 with real Oxigraph and Jena 6.2.0.
Input: [data.trig](data.trig). Config: [example-rules-config.json](../../example-rules-config.json).
Rules are [works-in.rq](works-in.rq), [second-pass.rq](second-pass.rq),
[line-total.rq](line-total.rq), and [work.rules](work.rules).

SPARQL jobs read the original snapshot once. Jena recursively chains the same
country relation into a continent relation. The second SPARQL rule cannot see
facts produced by the first rule. This is intentional, not missing inference.

```sh
cargo build -p rdf-cli
docker compose -f sidecars/jena-rules/compose.yml up --build -d
python3 plugins/inference/examples/rules/run.py --output target/my-rule-demo
```

Use `--sparql-only` to run the native provider without a sidecar. Existing demo
databases are never overwritten. Source graphs are explicitly selected; the secret
entity in the excluded graph contributes no deductions.

## sparql-countries

```sparql
SELECT ?person ?country WHERE { GRAPH <urn:rdftool:derived:inference:work-sparql> { ?person <https://example.org/worksInCountry> ?country } } ORDER BY ?person
```

```json
{
  "head": {
    "vars": [
      "person",
      "country"
    ]
  },
  "results": {
    "bindings": [
      {
        "person": {
          "type": "uri",
          "value": "https://example.org/alice"
        },
        "country": {
          "type": "uri",
          "value": "https://example.org/France"
        }
      },
      {
        "person": {
          "type": "uri",
          "value": "https://example.org/bob"
        },
        "country": {
          "type": "uri",
          "value": "https://example.org/USA"
        }
      }
    ]
  }
}
```

## sparql-no-second-pass

```sparql
ASK { GRAPH <urn:rdftool:derived:inference:work-sparql> { ?person <https://example.org/worksOnContinent> ?continent } }
```

```json
{
  "head": {},
  "boolean": false
}
```

## sparql-totals

```sparql
SELECT ?line ?total WHERE { GRAPH <urn:rdftool:derived:inference:totals> { ?line <https://example.org/total> ?total } }
```

```json
{
  "head": {
    "vars": [
      "line",
      "total"
    ]
  },
  "results": {
    "bindings": [
      {
        "line": {
          "type": "uri",
          "value": "https://example.org/line1"
        },
        "total": {
          "type": "literal",
          "value": "21",
          "datatype": "http://www.w3.org/2001/XMLSchema#integer"
        }
      }
    ]
  }
}
```

## jena-continents

```sparql
SELECT ?person ?continent WHERE { GRAPH <urn:rdftool:derived:inference:work-jena> { ?person <https://example.org/worksOnContinent> ?continent } } ORDER BY ?person
```

```json
{
  "head": {
    "vars": [
      "person",
      "continent"
    ]
  },
  "results": {
    "bindings": [
      {
        "person": {
          "type": "uri",
          "value": "https://example.org/alice"
        },
        "continent": {
          "type": "uri",
          "value": "https://example.org/Europe"
        }
      },
      {
        "person": {
          "type": "uri",
          "value": "https://example.org/bob"
        },
        "continent": {
          "type": "uri",
          "value": "https://example.org/NorthAmerica"
        }
      }
    ]
  }
}
```

