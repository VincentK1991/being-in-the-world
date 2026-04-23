use assert_cmd::Command;
use insta::assert_snapshot;

fn run_stdout(args: &[&str]) -> String {
    let output = Command::cargo_bin("rdftool")
        .expect("binary exists")
        .args(args)
        .output()
        .expect("command runs");
    assert!(
        output.status.success(),
        "command failed: status={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is utf8")
}

#[test]
fn snapshot_query_pattern_rdf_type() {
    let stdout = run_stdout(&[
        "query-pattern",
        "--source",
        "tests/fixtures/phase3/basic.ttl",
        "--subject",
        "?x",
        "--predicate",
        "rdf:type",
    ]);
    assert_snapshot!("query_pattern_rdf_type", stdout);
}

#[test]
fn snapshot_dump_nquads() {
    let stdout =
        run_stdout(&["dump", "--source", "tests/fixtures/phase3/basic.ttl", "--format", "n-quads"]);
    assert_snapshot!("dump_nquads", stdout);
}

#[test]
fn snapshot_stats_basic_ttl() {
    let stdout = run_stdout(&["stats", "--source", "tests/fixtures/phase3/basic.ttl"]);
    assert_snapshot!("stats_basic_ttl", stdout);
}
