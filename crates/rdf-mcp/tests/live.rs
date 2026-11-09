//! End to end over MCP against a running local stack (the demo data from `just seed`):
//! `cargo test -p rdf-mcp --test live -- --ignored`. `RDF_MCP_GATEWAY` overrides the gateway.
use rdf_mcp::{client::Gateway, tools::RdfMcp};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::time::Duration;

async fn client() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let base = std::env::var("RDF_MCP_GATEWAY").unwrap_or_else(|_| "http://127.0.0.1:7878".into());
    let gateway = Gateway::new(&base, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(server) = RdfMcp::new(gateway).serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    ().serve(client_io).await.unwrap_or_else(|e| panic!("{e}"))
}

async fn call(
    c: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> (bool, Value) {
    let Value::Object(args) = args else { panic!("arguments must be an object") };
    let result = c
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    (result.is_error.unwrap_or(false), result.structured_content.unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "needs the local stack"]
async fn tools_answer_against_the_demo_data() {
    let c = client().await;
    let tools: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for name in [
        "list_datasets",
        "describe_dataset",
        "get_schema",
        "list_endpoints",
        "list_plugins",
        "sparql_query",
        "search_entities",
        "describe_entity",
        "reasoning",
    ] {
        assert!(tools.contains(&name.to_owned()), "missing tool {name}");
    }

    let (error, out) = call(&c, "list_datasets", json!({})).await;
    assert!(
        !error
            && out["datasets"].as_array().is_some_and(|d| d.iter().any(|d| d["name"] == "default")),
        "{out}"
    );

    // Inferred facts are visible by default and tagged when described.
    let (error, out) = call(
        &c,
        "sparql_query",
        json!({"dataset": "default",
        "query": "ASK { <https://example.org/car1> a <https://example.org/Vehicle> }"}),
    )
    .await;
    assert!(!error && out["boolean"] == true, "{out}");
    let (_, out) = call(
        &c,
        "sparql_query",
        json!({"dataset": "default", "graphs": "asserted",
        "query": "ASK { <https://example.org/car1> a <https://example.org/Vehicle> }"}),
    )
    .await;
    assert_eq!(out["boolean"], false, "{out}");
    let (_, out) = call(
        &c,
        "describe_entity",
        json!({"dataset": "default", "iri": "https://example.org/car1"}),
    )
    .await;
    let origins: Vec<String> = out["outgoing"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t[2].as_str().map(str::to_owned))
        .collect();
    assert!(
        origins.iter().any(|o| o == "inferred vehicles")
            && origins.iter().any(|o| o.starts_with("asserted")),
        "{out}"
    );

    // Semantic search joined with graph patterns, across the merged graphs.
    let (error, out) = call(&c, "sparql_query", json!({"dataset": "default",
        "query": "SELECT ?s ?type WHERE { (?s ?p ?o ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"jewelry theft\" 2) . ?s a ?type }"})).await;
    assert!(!error && out["row_count"].as_u64().unwrap_or(0) > 0, "{out}");

    // A dry run names the misspelt term and the right one.
    let (error, out) = call(
        &c,
        "sparql_query",
        json!({"dataset": "default", "dry_run": true,
        "query": "SELECT ?c WHERE { ?c a <http://data.world/schema/insurance/Clam> }"}),
    )
    .await;
    let warnings = out["warnings"].to_string();
    assert!(!error && out["ran"] == false && warnings.contains("insurance/Claim>"), "{out}");

    // Read-only and allowlist are enforced.
    let (error, _) =
        call(&c, "sparql_query", json!({"dataset": "default", "query": "CLEAR ALL"})).await;
    assert!(error);
    let (error, out) = call(
        &c,
        "sparql_query",
        json!({"dataset": "default",
        "query": "SELECT * WHERE { SERVICE <http://evil.example/sparql> { ?s ?p ?o } }"}),
    )
    .await;
    assert!(error && out.to_string().contains("not an allowed endpoint"), "{out}");

    // The reasoner explains an inferred type.
    let (error, out) = call(&c, "reasoning", json!({"dataset": "default", "action": "explain", "job": "vehicles",
        "triple": "<https://example.org/car1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://example.org/Vehicle>"})).await;
    assert!(!error && out["derived"] == true, "{out}");

    let (error, out) = call(&c, "get_schema", json!({"dataset": "default", "focus": "<http://data.world/schema/insurance/Claim>", "samples": 1})).await;
    assert!(!error && out["class"]["instances"].as_u64().unwrap_or(0) > 0, "{out}");
    let _ = c.cancel().await;
}
