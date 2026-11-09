//! Worked examples: one SPARQL query composing several plugins, run through `sparql_query` over
//! MCP against the running, seeded stack. `cargo test -p rdf-mcp --test composition -- --ignored`
//! (`just mcp-test`). The same queries are shown in the docs page "Connect an AI agent (MCP)".
use rdf_mcp::{client::Gateway, tools::RdfMcp};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::time::Duration;

const ONTOP: &str = "http://ontop:8080/sparql";
const FINANCE: &str = "http://host.docker.internal:7878/d/finance/query";
const SELF: &str = "http://host.docker.internal:7878/query";

struct Example {
    name: &'static str,
    /// Which plugins the one query uses.
    plugins: &'static [&'static str],
    query: String,
    graphs: Option<Value>,
    /// Strings the answer must contain.
    expect: &'static [&'static str],
}

fn examples() -> Vec<Example> {
    vec![
        Example {
            name: "semantic search joined with stored and inferred types",
            plugins: &["semantic-search", "inference"],
            query: "SELECT DISTINCT ?policy ?type WHERE {
                      (?policy ?p ?note ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"jewelry theft\" 3) .
                      ?policy a ?type }"
                .into(),
            graphs: None,
            expect: &["insurance/Policy>"],
        },
        Example {
            name: "plugin catalog drives which inferred graphs to read",
            plugins: &["plugin-catalog", "inference"],
            query: "SELECT ?job (COUNT(*) AS ?triples) WHERE {
                      (?plugin ?kind ?job) <urn:rdftool:plugins:provides> () FILTER(?kind = \"job\")
                      BIND(IRI(CONCAT(\"urn:rdftool:derived:inference:\", ?job)) AS ?g)
                      GRAPH ?g { ?s ?p ?o } } GROUP BY ?job"
                .into(),
            graphs: None,
            expect: &["\"vehicles\"", "\"acme\""],
        },
        Example {
            name: "a custom property function generates the classes to count",
            plugins: &["custom-properties", "inference"],
            query: "PREFIX ins: <http://data.world/schema/insurance/>
                    SELECT ?local (COUNT(DISTINCT ?x) AS ?n) WHERE {
                      (?local) <urn:rdftool:property:split> (\"Claim,Policy\" \",\")
                      BIND(IRI(CONCAT(STR(ins:), ?local)) AS ?class) ?x a ?class } GROUP BY ?local"
                .into(),
            graphs: None,
            expect: &["\"Claim\"", "\"Policy\""],
        },
        Example {
            name: "the SQL virtual graph joined with local named graphs",
            plugins: &["virtual-graph", "federation"],
            query: format!(
                "PREFIX ins: <http://data.world/schema/insurance/>
                 SELECT DISTINCT ?policy ?number ?type WHERE {{
                   SERVICE <{ONTOP}> {{ ?policy ins:policyNumber ?number }}
                   GRAPH ?g {{ ?policy a ?type }} }}"
            ),
            graphs: None,
            expect: &["31003000336", "insurance/Policy>"],
        },
        Example {
            name: "query-time OWL 2 QL reasoning in the virtual graph",
            plugins: &["virtual-graph", "federation"],
            query: format!(
                "PREFIX ins: <http://data.world/schema/insurance/>
                 SELECT ?amount WHERE {{ SERVICE <{ONTOP}> {{ ?amount a ins:Reserve }} }}"
            ),
            graphs: None,
            expect: &["ClaimAmount"],
        },
        Example {
            name: "local semantic search joined with another dataset",
            plugins: &["semantic-search", "federation"],
            query: format!(
                "SELECT ?policy ?centre WHERE {{
                   (?policy ?p ?note ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"flood\" 1)
                   SERVICE <{FINANCE}> {{ ?policy <https://finance.example.org/vocab#costCentre> ?centre }} }}"
            ),
            graphs: None,
            expect: &["CC-100"],
        },
        Example {
            name: "SQL rows and another dataset in one answer",
            plugins: &["virtual-graph", "federation"],
            query: format!(
                "PREFIX ins: <http://data.world/schema/insurance/>
                 SELECT ?policy ?number ?centre WHERE {{
                   SERVICE <{ONTOP}> {{ ?policy ins:policyNumber ?number }}
                   SERVICE <{FINANCE}> {{ ?policy <https://finance.example.org/vocab#costCentre> ?centre }} }}"
            ),
            graphs: None,
            expect: &["31003000336", "CC-200"],
        },
        Example {
            name: "a property function evaluated by the SERVICE endpoint",
            plugins: &["federation", "semantic-search"],
            query: format!(
                "SELECT ?policy ?score WHERE {{ SERVICE <{SELF}> {{
                   (?policy ?p ?note ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"flood\" 2) }} }}"
            ),
            graphs: None,
            expect: &["Policy-1>"],
        },
        Example {
            name: "a SHACL report joined with the data it judged",
            plugins: &["shacl"],
            query: "PREFIX sh: <http://www.w3.org/ns/shacl#>
                    SELECT ?focus ?property ?value ?message WHERE {
                      GRAPH <urn:rdftool:derived:shacl:claims-example> {
                        ?r sh:focusNode ?focus ; sh:resultSeverity sh:Violation ; sh:resultMessage ?message ; sh:resultPath ?property }
                      OPTIONAL { GRAPH <urn:shacl:example:data> { ?focus ?property ?value } } }"
                .into(),
            graphs: None,
            expect: &["-10", "nonnegative"],
        },
        Example {
            name: "CONSTRUCT a graph from search hits and their types",
            plugins: &["semantic-search", "inference"],
            query: "CONSTRUCT { ?policy <urn:example:mentions> ?note ; a ?type } WHERE {
                      (?policy ?p ?note ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"jewelry theft\" 1) .
                      ?policy a ?type }"
                .into(),
            graphs: None,
            expect: &["<urn:example:mentions>", "Jewelry"],
        },
        Example {
            name: "only what the reasoner added",
            plugins: &["inference"],
            query: "SELECT ?type WHERE { <https://example.org/car1> a ?type }".into(),
            graphs: Some(json!("inferred")),
            expect: &["<https://example.org/Vehicle>", "<https://example.org/Asset>"],
        },
    ]
}

/// Every string and number in the answer, one per line, so expectations match terms as written.
fn leaves(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{s}\n"),
        Value::Array(items) => items.iter().map(leaves).collect(),
        Value::Object(map) => map.values().map(leaves).collect(),
        other => format!("{other}\n"),
    }
}

#[tokio::test]
#[ignore = "needs the local stack"]
async fn one_query_composes_several_plugins() {
    let base = std::env::var("RDF_MCP_GATEWAY").unwrap_or_else(|_| "http://127.0.0.1:7878".into());
    let gateway = Gateway::new(&base, Duration::from_secs(90)).unwrap_or_else(|e| panic!("{e}"));
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(server) = RdfMcp::new(gateway).serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    let client = ().serve(client_io).await.unwrap_or_else(|e| panic!("{e}"));

    let mut failures = Vec::new();
    for example in examples() {
        let mut args = json!({"dataset": "default", "query": example.query});
        if let Some(graphs) = &example.graphs {
            args["graphs"] = graphs.clone();
        }
        let Value::Object(args) = args else { unreachable!("built as an object") };
        // A dry run first: it must accept the query and find nothing wrong.
        let mut dry = args.clone();
        dry.insert("dry_run".into(), json!(true));
        let checked =
            client.call_tool(CallToolRequestParams::new("sparql_query").with_arguments(dry)).await;
        let result =
            client.call_tool(CallToolRequestParams::new("sparql_query").with_arguments(args)).await;
        let (checked, result) = match (checked, result) {
            (Ok(c), Ok(r)) => (c.structured_content.unwrap_or_default(), r),
            (c, r) => {
                failures.push(format!("{}: transport error {c:?} {r:?}", example.name));
                continue;
            }
        };
        let answer = result.structured_content.unwrap_or_default();
        let text = leaves(&answer);
        let problems: Vec<String> = [
            (result.is_error == Some(true)).then(|| format!("error {text}")),
            (checked["ok"] != true).then(|| format!("dry run rejected it: {checked}")),
            checked["warnings"]
                .as_array()
                .filter(|w| !w.is_empty())
                .map(|w| format!("dry run warned: {w:?}")),
        ]
        .into_iter()
        .flatten()
        .chain(
            example
                .expect
                .iter()
                .filter(|e| !text.contains(*e))
                .map(|e| format!("missing {e} in {text}")),
        )
        .collect();
        if !problems.is_empty() {
            failures.push(format!(
                "{} ({}): {}",
                example.name,
                example.plugins.join(" + "),
                problems.join("; ")
            ));
        }
    }
    let _ = client.cancel().await;
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
