#![allow(clippy::unwrap_used)] // test code: a failed unwrap is the failure report
//! End-to-end: one gateway, several isolated datasets, each served by its own child process.
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Gateway(Child);
impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = Command::new("kill").args(["-TERM", &self.0.id().to_string()]).status();
        let _ = self.0.wait();
    }
}

fn start(root: &std::path::Path) -> (Gateway, String, Client) {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new(env!("CARGO_BIN_EXE_rdftool"))
        .args(["gateway", "--db"])
        .arg(root)
        .args(["--bind", &format!("127.0.0.1:{port}")])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let client = Client::builder().no_proxy().timeout(Duration::from_secs(60)).build().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while client.get(format!("{base}/health")).send().is_err() {
        assert!(Instant::now() < deadline, "gateway did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    (Gateway(child), base, client)
}

fn names(client: &Client, base: &str) -> Vec<String> {
    let listing: Value = client.get(format!("{base}/api/datasets")).send().unwrap().json().unwrap();
    listing["datasets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().into())
        .collect()
}
fn count(client: &Client, url: String) -> u64 {
    let response = client
        .post(url)
        .header("Content-Type", "application/sparql-query")
        .header("Accept", "application/sparql-results+json")
        .body("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }")
        .send()
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().unwrap();
    body["results"]["bindings"][0]["n"]["value"].as_str().unwrap().parse().unwrap()
}

#[test]
fn datasets_are_isolated_listed_created_and_deleted() {
    let root = tempfile::tempdir().unwrap();
    let (_gateway, base, client) = start(root.path());
    assert_eq!(names(&client, &base), ["default"]);

    for name in ["alpha", "beta"] {
        let created = client
            .post(format!("{base}/api/datasets"))
            .json(&json!({"name": name}))
            .send()
            .unwrap();
        assert_eq!(created.status(), 201, "{name}");
    }
    assert_eq!(
        client
            .post(format!("{base}/api/datasets"))
            .json(&json!({"name": "alpha"}))
            .send()
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        client
            .post(format!("{base}/api/datasets"))
            .json(&json!({"name": "../x"}))
            .send()
            .unwrap()
            .status(),
        400
    );
    assert_eq!(names(&client, &base), ["alpha", "beta", "default"]);

    let import = client
        .post(format!("{base}/d/alpha/api/import"))
        .header("Content-Type", "text/turtle")
        .body("<urn:a> <urn:p> <urn:b> , <urn:c> .")
        .send()
        .unwrap();
    assert!(import.status().is_success(), "{import:?}");
    assert_eq!(count(&client, format!("{base}/d/alpha/query")), 2);
    assert_eq!(count(&client, format!("{base}/d/beta/query")), 0);
    assert_eq!(
        count(&client, format!("{base}/query")),
        0,
        "unprefixed requests use the default dataset"
    );
    assert_eq!(client.get(format!("{base}/d/missing/health")).send().unwrap().status(), 404);

    let listing: Value = client.get(format!("{base}/api/datasets")).send().unwrap().json().unwrap();
    let alpha =
        listing["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "alpha").unwrap();
    assert_eq!(alpha["running"], true);
    assert_eq!(alpha["quads"], 2);

    assert_eq!(client.delete(format!("{base}/api/datasets/default")).send().unwrap().status(), 409);
    assert_eq!(client.delete(format!("{base}/api/datasets/nope")).send().unwrap().status(), 404);
    assert!(
        client.delete(format!("{base}/api/datasets/alpha")).send().unwrap().status().is_success()
    );
    assert_eq!(names(&client, &base), ["beta", "default"]);
    assert_eq!(client.get(format!("{base}/d/alpha/health")).send().unwrap().status(), 404);
    assert_eq!(count(&client, format!("{base}/d/beta/query")), 0);
}

#[test]
fn cli_lists_creates_and_deletes_datasets() {
    let root = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rdftool"))
            .arg("datasets")
            .arg("--db")
            .arg(root.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["create", "one"]).status.success());
    assert!(!run(&["create", "one"]).status.success(), "duplicate create fails");
    assert!(!run(&["create", "bad/name"]).status.success());
    let listed: Value = serde_json::from_slice(&run(&["list"]).stdout).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!run(&["delete", "one"]).status.success(), "delete needs --yes");
    assert!(run(&["delete", "one", "--yes"]).status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&run(&["list"]).stdout).unwrap().as_array().unwrap().len(),
        0
    );
}
