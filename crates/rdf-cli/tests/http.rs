use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn live_http_query_update_graph_store_and_reopen() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let binary = assert_cmd::cargo::cargo_bin("rdftool");
    let mut server = Server(
        Command::new(&binary)
            .args([
                "serve",
                "--db",
                temp.path().to_str().expect("path"),
                "--bind",
                &address.to_string(),
                "--max-body-bytes",
                "1024",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)).build()?;
    let base = format!("http://{address}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if client.get(format!("{base}/health")).send().is_ok() {
            break;
        }
        assert!(server.0.try_wait()?.is_none(), "server exited before ready");
        assert!(Instant::now() < deadline, "server readiness timeout");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        client
            .post(format!("{base}/update"))
            .header("Content-Type", "application/sparql-update")
            .body("INSERT DATA { <urn:s> <urn:p> 1 }")
            .send()?
            .status(),
        204
    );
    let response = client
        .get(format!("{base}/query"))
        .query(&[("query", "ASK { <urn:s> <urn:p> 1 }")])
        .send()?;
    assert_eq!(response.status(), 200);
    let result: serde_json::Value = serde_json::from_str(&response.text()?)?;
    assert_eq!(result["boolean"], true);
    assert_eq!(
        client
            .post(format!("{base}/query"))
            .header("Content-Type", "application/sparql-query")
            .body("bad query")
            .send()?
            .status(),
        400
    );
    assert_eq!(client.post(format!("{base}/query")).body("ASK {}").send()?.status(), 415);
    assert_eq!(
        client
            .post(format!("{base}/query"))
            .header("Content-Type", "application/sparql-query")
            .body("x".repeat(2048))
            .send()?
            .status(),
        413
    );
    let graph = format!("{base}/store?graph=urn:g");
    assert_eq!(
        client
            .put(&graph)
            .header("Content-Type", "text/turtle")
            .body("<urn:a> <urn:p> 2 .")
            .send()?
            .status(),
        201
    );
    assert_eq!(
        client.put(&graph).header("Content-Type", "text/turtle").body("invalid").send()?.status(),
        400
    );
    let response = client.get(&graph).header("Accept", "application/n-triples").send()?;
    assert_eq!(response.status(), 200);
    assert!(response.text()?.contains("<urn:a>"));
    assert_eq!(client.delete(&graph).send()?.status(), 204);
    assert_eq!(client.get(&graph).send()?.status(), 404);
    let output = Command::new(&binary)
        .args(["query", "--source", &base, "ASK { <urn:s> <urn:p> 1 }"])
        .output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stdout)?["boolean"], true);
    let page = client.get(&base).send()?;
    if page.status() == 200 {
        let html = page.text()?;
        assert!(html.contains("Graph explorer"), "the front page is the graph explorer");
        assert!(html.contains("/_astro/"));
    } else {
        // An API-only Rust build deliberately does not require a Node toolchain.
        assert_eq!(page.status(), 503);
        assert!(page.text()?.contains("Studio assets not built"));
    }
    let plugins: serde_json::Value =
        serde_json::from_str(&client.get(format!("{base}/api/plugins")).send()?.text()?)?;
    assert!(plugins.as_array().expect("plugins").iter().any(|p| p["id"] == "dataset-summary"));
    let custom = plugins
        .as_array()
        .expect("plugins")
        .iter()
        .find(|p| p["id"] == "custom-properties")
        .expect("custom properties");
    assert_eq!(custom["sdk_version"], 3);
    // The server describes its own plugins to federation engines (Service Description) ...
    let description = client.get(format!("{base}/query")).send()?.text()?;
    assert!(description.contains("sd:Service"), "{description}");
    assert!(description.contains("rdftool:id \"custom-properties\""), "{description}");
    assert!(description.contains("rdftool:propertyFunction <urn:rdftool:property:tokens>"));
    // ... and to queries, from the same catalog.
    let listed = client
        .get(format!("{base}/query"))
        .query(&[(
            "query",
            "SELECT ?p WHERE { (?p ?v) <urn:rdftool:plugins:available> () } ORDER BY ?p",
        )])
        .header("Accept", "application/sparql-results+json")
        .send()?
        .text()?;
    assert!(listed.contains("plugin-catalog") && listed.contains("custom-properties"), "{listed}");
    assert_eq!(custom["properties"].as_array().expect("properties").len(), 2);
    let property_query = "SELECT ?token ?position WHERE { (?token ?position) <urn:rdftool:property:tokens> 'hello world' } ORDER BY ?position";
    let response = client
        .post(format!("{base}/query"))
        .header("Content-Type", "application/sparql-query")
        .body(property_query)
        .send()?;
    assert_eq!(response.status(), 200);
    let property_result: serde_json::Value = serde_json::from_str(&response.text()?)?;
    assert_eq!(property_result["results"]["bindings"][1]["token"]["value"], "world");
    let cli = Command::new(&binary).args(["query", "--source", &base, property_query]).output()?;
    assert!(cli.status.success());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&cli.stdout)?, property_result);
    let response = client
        .post(format!("{base}/query"))
        .header("Content-Type", "application/sparql-query")
        .body("SELECT * WHERE { ?x <urn:rdftool:property:split> (?text ',') }")
        .send()?;
    assert_eq!(response.status(), 400);
    assert!(response.text()?.contains("constants"));
    let operation = format!("{base}/api/plugins/dataset-summary/operations/count");
    let response =
        client.post(&operation).header("Content-Type", "application/json").body("{}").send()?;
    assert_eq!(response.status(), 200);
    let count: serde_json::Value = serde_json::from_str(&response.text()?)?;
    assert_eq!(count["results"]["bindings"][0]["quads"]["value"], "1");
    assert_eq!(
        client
            .post(&operation)
            .header("Content-Type", "application/json")
            .body("{")
            .send()?
            .status(),
        400
    );
    assert_eq!(
        client.post(&operation).header("Content-Type", "text/plain").body("{}").send()?.status(),
        415
    );
    assert_eq!(
        client
            .post(format!("{base}/api/plugins/missing/operations/count"))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()?
            .status(),
        404
    );
    let upload = client
        .post(format!("{base}/api/import"))
        .header("Content-Type", "text/turtle")
        .body("@prefix ex: <https://example.org/> . ex:a ex:name \"Alice\" ; ex:knows ex:b .")
        .send()?;
    assert_eq!(upload.status(), 200);
    let uploaded: serde_json::Value = serde_json::from_str(&upload.text()?)?;
    assert_eq!(uploaded["parsed_quads"], 2);
    assert_eq!(uploaded["total_quads"], 3);
    let invalid = client
        .post(format!("{base}/api/import?graph=https%3A%2F%2Fexample.org%2Fbad"))
        .header("Content-Type", "text/turtle")
        .body("<urn:transient> <urn:p> 1 . invalid")
        .send()?;
    assert_eq!(invalid.status(), 400);
    let status: serde_json::Value =
        serde_json::from_str(&client.get(format!("{base}/api/status")).send()?.text()?)?;
    assert_eq!(status["quads"], 3);
    assert_eq!(status["graphs"].as_array().expect("graphs").len(), 0);
    assert_eq!(status["max_upload_bytes"], 1024);
    let named = client
        .post(format!("{base}/api/import?graph=https%3A%2F%2Fexample.org%2Fpeople"))
        .header("Content-Type", "text/turtle")
        .body("<https://example.org/a> <https://example.org/name> \"Alice\" .")
        .send()?;
    assert_eq!(named.status(), 200);
    let query = client.post(format!("{base}/query"))
        .header("Content-Type", "application/sparql-query")
        .body("SELECT ?name WHERE { GRAPH <https://example.org/people> { ?s <https://example.org/name> ?name } }")
        .send()?;
    let queried: serde_json::Value = serde_json::from_str(&query.text()?)?;
    assert_eq!(queried["results"]["bindings"][0]["name"]["value"], "Alice");
    assert_eq!(
        client
            .post(format!("{base}/api/import"))
            .header("Content-Type", "text/turtle")
            .body("x".repeat(2048))
            .send()?
            .status(),
        413
    );
    assert_eq!(
        client
            .post(format!("{base}/api/import"))
            .header("Content-Type", "text/plain")
            .body("data")
            .send()?
            .status(),
        415
    );
    assert_eq!(
        client
            .post(format!("{base}/api/import"))
            .header("Content-Type", "text/turtle")
            .body("")
            .send()?
            .status(),
        400
    );
    server.0.kill()?;
    server.0.wait()?;
    assert_eq!(rdf_store::engine::Dataset::open(temp.path(), "default", false)?.store().len()?, 4);
    Ok(())
}
