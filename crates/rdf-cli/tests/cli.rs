use assert_cmd::Command;
use predicates::prelude::*;
use rdf_store::{
    engine::Dataset,
    oxigraph::io::{RdfFormat, RdfParser},
};
use std::{fs, path::Path};
fn run(args: &[&str]) -> Vec<u8> {
    let output = Command::cargo_bin("rdftool").expect("binary").args(args).output().expect("run");
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}
fn path(path: &Path) -> &str {
    path.to_str().expect("path")
}
const BASIC: &str = "tests/fixtures/phase3/basic.ttl";
#[test]
fn property_queries_match_file_database_and_reopen() {
    let temp = tempfile::tempdir().expect("temp");
    let db = temp.path().join("db");
    run(&["ingest", "--from", BASIC, "--to", path(&db)]);
    let query = "SELECT ?word ?position WHERE { (?word ?position) <urn:rdftool:property:tokens> 'hello world' } ORDER BY ?position";
    let file: serde_json::Value =
        serde_json::from_slice(&run(&["query", "--source", BASIC, query])).expect("json");
    let persistent: serde_json::Value =
        serde_json::from_slice(&run(&["query", "--source", path(&db), query])).expect("json");
    assert_eq!(file, persistent);
    assert_eq!(persistent["results"]["bindings"][1]["word"]["value"], "world");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&run(&["query", "--source", path(&db), query]))
            .expect("reopened json"),
        persistent
    );
    let query_file = temp.path().join("property.rq");
    fs::write(&query_file, query).expect("query");
    run(&["diff", "--query", path(&query_file), "--sources", &format!("{BASIC},{}", path(&db))]);
    Command::cargo_bin("rdftool").expect("binary").args(["update", "--source", path(&db), "INSERT { <urn:s> <urn:p> ?word } WHERE { ?word <urn:rdftool:property:split> ('a' ',') }"]).assert().failure().stderr(predicate::str::contains("query-only"));
}
#[test]
fn help_lists_core_subcommands() {
    let help = String::from_utf8(run(&["--help"])).expect("utf8");
    for name in [
        "ingest",
        "query",
        "query-pattern",
        "dump",
        "stats",
        "diff",
        "test-suite",
        "smoke-suite",
        "serve",
        "backup",
        "restore",
        "migrate",
        "update",
    ] {
        assert!(help.contains(name));
    }
}
#[test]
fn ingest_formats_and_reject_malformed_inputs() {
    let temp = tempfile::tempdir().expect("temp");
    let output = temp.path().join("out.nq");
    for (positive, negative) in [
        ("w3c-ntriples/positive/01-simple.pos.nt", "w3c-ntriples/negative/01-missing-dot.neg.nt"),
        ("w3c-nquads/positive/01-default.pos.nq", "w3c-nquads/negative/01-missing-dot.neg.nq"),
        ("w3c-turtle/positive/01-prefix.pos.ttl", "w3c-turtle/negative/01-unknown-prefix.neg.ttl"),
        (
            "w3c-trig/positive/01-simple-graph.pos.trig",
            "w3c-trig/negative/01-missing-rbrace.neg.trig",
        ),
    ] {
        assert!(
            run(&[
                "ingest",
                "--from",
                &format!("tests/fixtures/{positive}"),
                "--to",
                path(&output)
            ])
            .is_empty()
        );
        let bytes = fs::read(&output).expect("output");
        assert_eq!(
            RdfParser::from_format(RdfFormat::NQuads)
                .for_reader(bytes.as_slice())
                .collect::<Result<Vec<_>, _>>()
                .expect("parse")
                .len(),
            1
        );
        Command::cargo_bin("rdftool")
            .expect("binary")
            .args([
                "ingest",
                "--from",
                &format!("tests/fixtures/{negative}"),
                "--to",
                path(&output),
            ])
            .assert()
            .failure();
        assert_eq!(fs::read(&output).expect("unchanged"), bytes);
    }
}
#[test]
fn file_conversion_and_machine_output() {
    let temp = tempfile::tempdir().expect("temp");
    for extension in ["nt", "nq"] {
        let output = temp.path().join(format!("out.{extension}"));
        run(&["ingest", "--from", BASIC, "--to", path(&output)]);
        let dump = run(&["dump", "--source", path(&output), "--format", "nquads"]);
        assert_eq!(
            RdfParser::from_format(RdfFormat::NQuads)
                .for_reader(dump.as_slice())
                .collect::<Result<Vec<_>, _>>()
                .expect("parse")
                .len(),
            2
        );
    }
    let stats: serde_json::Value =
        serde_json::from_slice(&run(&["stats", "--source", BASIC])).expect("JSON");
    assert_eq!(stats["quads"], 2);
    assert_eq!(stats["distinct_predicates"], 2);
}
#[test]
fn persistent_parity_isolation_and_idempotence() {
    let temp = tempfile::tempdir().expect("temp");
    let db = temp.path().join("db");
    run(&["ingest", "--from", BASIC, "--to", path(&db), "--dataset", "a"]);
    run(&["ingest", "--from", BASIC, "--to", path(&db), "--dataset", "a"]);
    drop(Dataset::open(&db, "b", true).expect("empty dataset"));
    let file = run(&["query-pattern", "--source", BASIC, "--predicate", "rdf:type"]);
    let persisted =
        run(&["query-pattern", "--source", path(&db), "--dataset", "a", "--predicate", "rdf:type"]);
    assert_eq!(file, persisted);
    assert_eq!(file.iter().filter(|b| **b == b'\n').count(), 1);
    assert!(run(&["query-pattern", "--source", path(&db), "--dataset", "b"]).is_empty());
    let stats: serde_json::Value =
        serde_json::from_slice(&run(&["stats", "--source", path(&db), "--dataset", "a"]))
            .expect("JSON");
    assert_eq!(stats["quads"], 2);
    assert_eq!(
        run(&["dump", "--source", path(&db), "--dataset", "a"]),
        run(&["dump", "--source", BASIC])
    );
}
#[test]
fn query_update_backup_restore_and_diff() {
    let temp = tempfile::tempdir().expect("temp");
    let db = temp.path().join("db");
    let backup = temp.path().join("backup");
    let restored = temp.path().join("restored");
    run(&["ingest", "--from", BASIC, "--to", path(&db)]);
    run(&["update", "--source", path(&db), "INSERT DATA { <urn:s> <urn:p> 42 }"]);
    let result: serde_json::Value = serde_json::from_slice(&run(&[
        "query",
        "--source",
        path(&db),
        "ASK { <urn:s> <urn:p> 42 }",
    ]))
    .expect("JSON");
    assert_eq!(result["boolean"], true);
    run(&["backup", "--source", path(&db), "--to", path(&backup)]);
    run(&["restore", "--from", path(&backup), "--to", path(&restored)]);
    let query = temp.path().join("query.rq");
    fs::write(&query, "SELECT * WHERE { ?s ?p ?o } ORDER BY ?s").expect("query");
    assert_eq!(
        run(&[
            "diff",
            "--query",
            path(&query),
            "--sources",
            &format!("{},{}", path(&db), path(&restored))
        ]),
        b"equal\n"
    );
    Command::cargo_bin("rdftool")
        .expect("binary")
        .args(["restore", "--from", path(&backup), "--to", path(&restored)])
        .assert()
        .failure();
    Command::cargo_bin("rdftool")
        .expect("binary")
        .args(["update", "--source", BASIC, "CLEAR ALL"])
        .assert()
        .failure();
}
#[test]
fn real_manifest_and_explicit_smoke() {
    let output = run(&["test-suite", "--manifest", "tests/fixtures/conformance/manifest.ttl"]);
    let report: serde_json::Value = serde_json::from_slice(&output).expect("report");
    assert_eq!(report["passed"], 2);
    assert_eq!(report["failed"], 0);
    assert_eq!(report["unsupported"], 0);
    Command::cargo_bin("rdftool")
        .expect("binary")
        .args(["test-suite", "--manifest", "tests/fixtures/w3c-manifest"])
        .assert()
        .failure();
    Command::cargo_bin("rdftool")
        .expect("binary")
        .args(["smoke-suite", "--manifest", "tests/fixtures/w3c-manifest"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Passed"));
}
