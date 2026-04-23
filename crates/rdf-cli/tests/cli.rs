use assert_cmd::Command;
use predicates::prelude::*;
use rdf_io::{parse_nquads, parse_ntriples};
use std::fs;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn help_lists_core_subcommands() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest"))
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("query-pattern"))
        .stdout(predicate::str::contains("dump"))
        .stdout(predicate::str::contains("stats"))
        .stdout(predicate::str::contains("diff"))
        .stdout(predicate::str::contains("test-suite"))
        .stdout(predicate::str::contains("serve"));
}

fn unique_temp_path(ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("rdftool-phase3-{nanos}.{ext}"))
}

fn unique_db_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("rdftool-phase4-{nanos}.redb"))
}

fn run_stdout(args: &[&str]) -> String {
    let binary = assert_cmd::cargo::cargo_bin("rdftool");
    let output = ProcessCommand::new(binary).args(args).output().expect("run rdftool");
    assert!(
        output.status.success(),
        "command failed: {}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout utf-8")
}

#[test]
fn test_suite_reports_manifest_smoke_status_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["test-suite", "--manifest", "tests/fixtures/w3c-manifest", "--source", "fresh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("discovered=2"))
        .stdout(predicate::str::contains("passed=2"))
        .stdout(predicate::str::contains("failed=0"))
        .stdout(predicate::str::contains("pending=0"));
}

#[test]
fn test_suite_reports_ntriples_conformance_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["test-suite", "--manifest", "tests/fixtures/w3c-ntriples", "--source", "fresh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("discovered=4"))
        .stdout(predicate::str::contains("passed=4"))
        .stdout(predicate::str::contains("failed=0"));
}

#[test]
fn test_suite_reports_nquads_conformance_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["test-suite", "--manifest", "tests/fixtures/w3c-nquads", "--source", "fresh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("discovered=4"))
        .stdout(predicate::str::contains("passed=4"))
        .stdout(predicate::str::contains("failed=0"));
}

#[test]
fn test_suite_reports_turtle_conformance_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["test-suite", "--manifest", "tests/fixtures/w3c-turtle", "--source", "fresh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("discovered=4"))
        .stdout(predicate::str::contains("passed=4"))
        .stdout(predicate::str::contains("failed=0"));
}

#[test]
fn test_suite_reports_trig_conformance_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["test-suite", "--manifest", "tests/fixtures/w3c-trig", "--source", "fresh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("discovered=4"))
        .stdout(predicate::str::contains("passed=4"))
        .stdout(predicate::str::contains("failed=0"));
}

#[test]
fn ingest_parses_ntriples_input_and_reports_count() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-ntriples/positive/01-simple.pos.nt",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=ntriples"))
        .stdout(predicate::str::contains("parsed_triples=1"));
}

#[test]
fn ingest_fails_on_invalid_ntriples_input() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-ntriples/negative/01-missing-dot.neg.nt",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("line=1"))
        .stderr(predicate::str::contains("column="));
}

#[test]
fn ingest_parses_nquads_input_and_reports_count() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-nquads/positive/01-default.pos.nq",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=nquads"))
        .stdout(predicate::str::contains("parsed_quads=1"));
}

#[test]
fn ingest_parses_turtle_input_and_reports_count() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-turtle/positive/01-prefix.pos.ttl",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=turtle"))
        .stdout(predicate::str::contains("parsed_quads=1"));
}

#[test]
fn ingest_converts_turtle_to_nquads_output_file() {
    let output = unique_temp_path("nq");
    let output_str = output.to_string_lossy().into_owned();

    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["ingest", "--from", "tests/fixtures/phase3/basic.ttl", "--to", &output_str]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=turtle"))
        .stdout(predicate::str::contains("written_quads=2"));

    let written = fs::read_to_string(&output).expect("ingest wrote output file");
    let quads = parse_nquads(&written).expect("output is valid nquads");
    assert_eq!(quads.len(), 2);

    let _ = fs::remove_file(output);
}

#[test]
fn ingest_converts_turtle_to_ntriples_output_file() {
    let output = unique_temp_path("nt");
    let output_str = output.to_string_lossy().into_owned();

    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["ingest", "--from", "tests/fixtures/phase3/basic.ttl", "--to", &output_str]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=turtle"))
        .stdout(predicate::str::contains("written_quads=2"));

    let written = fs::read_to_string(&output).expect("ingest wrote output file");
    let triples = parse_ntriples(&written).expect("output is valid ntriples");
    assert_eq!(triples.len(), 2);

    let _ = fs::remove_file(output);
}

#[test]
fn ingest_parses_trig_input_and_reports_count() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-trig/positive/01-simple-graph.pos.trig",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ingest: source_format=trig"))
        .stdout(predicate::str::contains("parsed_quads=1"));
}

#[test]
fn ingest_fails_on_invalid_nquads_input() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-nquads/negative/01-missing-dot.neg.nq",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("line=1"))
        .stderr(predicate::str::contains("column="));
}

#[test]
fn ingest_fails_on_invalid_turtle_input() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-turtle/negative/01-unknown-prefix.neg.ttl",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert().failure().stderr(predicate::str::contains("kind=unknown prefix"));
}

#[test]
fn ingest_fails_on_invalid_trig_input() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "ingest",
        "--from",
        "tests/fixtures/w3c-trig/negative/01-missing-rbrace.neg.trig",
        "--to",
        "/tmp/out.nq",
    ]);

    cmd.assert().failure().stderr(predicate::str::contains("kind=unexpected end of input"));
}

#[test]
fn query_pattern_matches_rdf_type_in_turtle_source() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args([
        "query-pattern",
        "--source",
        "tests/fixtures/phase3/basic.ttl",
        "--subject",
        "?x",
        "--predicate",
        "rdf:type",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("matches=1"))
        .stdout(predicate::str::contains("<http://example.com/alice>"))
        .stdout(predicate::str::contains("<http://example.com/Person>"));
}

#[test]
fn dump_outputs_nquads_for_turtle_source() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["dump", "--source", "tests/fixtures/phase3/basic.ttl", "--format", "n-quads"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "dump: source=tests/fixtures/phase3/basic.ttl format=nquads quads=2",
        ))
        .stdout(predicate::str::contains("<http://example.com/alice>"));
}

#[test]
fn stats_reports_quad_and_distinct_predicate_counts() {
    let mut cmd = Command::cargo_bin("rdftool").expect("binary exists");
    cmd.args(["stats", "--source", "tests/fixtures/phase3/basic.ttl"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("stats: source=tests/fixtures/phase3/basic.ttl"))
        .stdout(predicate::str::contains("quads=2"))
        .stdout(predicate::str::contains("distinct_predicates=2"));
}

#[test]
fn ingest_writes_to_db_source_and_query_pattern_reads_it() {
    let db_path = unique_db_path();
    let db_path_str = db_path.to_string_lossy().into_owned();

    let mut ingest = Command::cargo_bin("rdftool").expect("binary exists");
    ingest.args([
        "ingest",
        "--from",
        "tests/fixtures/phase3/basic.ttl",
        "--to",
        &db_path_str,
        "--dataset",
        "dataset_a",
    ]);
    ingest
        .assert()
        .success()
        .stdout(predicate::str::contains("target_type=db"))
        .stdout(predicate::str::contains("written_quads=2"));

    let mut query = Command::cargo_bin("rdftool").expect("binary exists");
    query.args([
        "query-pattern",
        "--source",
        &db_path_str,
        "--dataset",
        "dataset_a",
        "--predicate",
        "rdf:type",
    ]);
    query
        .assert()
        .success()
        .stdout(predicate::str::contains("matches=1"))
        .stdout(predicate::str::contains("<http://example.com/alice>"));

    let _ = fs::remove_file(db_path);
}

#[test]
fn db_source_isolates_query_results_by_dataset() {
    let db_path = unique_db_path();
    let db_path_str = db_path.to_string_lossy().into_owned();

    let mut ingest = Command::cargo_bin("rdftool").expect("binary exists");
    ingest.args([
        "ingest",
        "--from",
        "tests/fixtures/phase3/basic.ttl",
        "--to",
        &db_path_str,
        "--dataset",
        "dataset_a",
    ]);
    ingest.assert().success();

    let mut query_a = Command::cargo_bin("rdftool").expect("binary exists");
    query_a.args([
        "query-pattern",
        "--source",
        &db_path_str,
        "--dataset",
        "dataset_a",
        "--predicate",
        "rdf:type",
    ]);
    query_a.assert().success().stdout(predicate::str::contains("matches=1"));

    let mut query_b = Command::cargo_bin("rdftool").expect("binary exists");
    query_b.args([
        "query-pattern",
        "--source",
        &db_path_str,
        "--dataset",
        "dataset_b",
        "--predicate",
        "rdf:type",
    ]);
    query_b.assert().success().stdout(predicate::str::contains("matches=0"));

    let _ = fs::remove_file(db_path);
}

#[test]
fn dump_and_stats_support_db_source() {
    let db_path = unique_db_path();
    let db_path_str = db_path.to_string_lossy().into_owned();

    let mut ingest = Command::cargo_bin("rdftool").expect("binary exists");
    ingest.args([
        "ingest",
        "--from",
        "tests/fixtures/phase3/basic.ttl",
        "--to",
        &db_path_str,
        "--dataset",
        "dataset_a",
    ]);
    ingest.assert().success();

    let mut dump = Command::cargo_bin("rdftool").expect("binary exists");
    dump.args(["dump", "--source", &db_path_str, "--dataset", "dataset_a", "--format", "n-quads"]);
    dump.assert()
        .success()
        .stdout(predicate::str::contains("dump: source="))
        .stdout(predicate::str::contains("quads=2"))
        .stdout(predicate::str::contains("<http://example.com/alice>"));

    let mut stats = Command::cargo_bin("rdftool").expect("binary exists");
    stats.args(["stats", "--source", &db_path_str, "--dataset", "dataset_a"]);
    stats
        .assert()
        .success()
        .stdout(predicate::str::contains("stats: source="))
        .stdout(predicate::str::contains("quads=2"))
        .stdout(predicate::str::contains("distinct_predicates=2"));

    let _ = fs::remove_file(db_path);
}

#[test]
fn query_pattern_parity_between_file_and_db_source() {
    let db_path = unique_db_path();
    let db_path_str = db_path.to_string_lossy().into_owned();

    let mut ingest = Command::cargo_bin("rdftool").expect("binary exists");
    ingest.args([
        "ingest",
        "--from",
        "tests/fixtures/phase3/basic.ttl",
        "--to",
        &db_path_str,
        "--dataset",
        "dataset_a",
    ]);
    ingest.assert().success();

    let file_output = run_stdout(&[
        "query-pattern",
        "--source",
        "tests/fixtures/phase3/basic.ttl",
        "--predicate",
        "rdf:type",
    ]);
    let db_output = run_stdout(&[
        "query-pattern",
        "--source",
        &db_path_str,
        "--dataset",
        "dataset_a",
        "--predicate",
        "rdf:type",
    ]);

    let file_lines: Vec<&str> = file_output.lines().skip(1).collect();
    let db_lines: Vec<&str> = db_output.lines().skip(1).collect();
    assert_eq!(file_lines, db_lines);

    let _ = fs::remove_file(db_path);
}
