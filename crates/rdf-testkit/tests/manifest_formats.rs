use rdf_testkit::manifest::{
    ManifestStatus, run_manifest_nquads, run_manifest_trig, run_manifest_turtle,
};

#[test]
fn nquads_runner_enforces_positive_and_negative_expectations() {
    let results = run_manifest_nquads("tests/fixtures/w3c-nquads").expect("run n-quads suite");
    assert_eq!(results.len(), 10);
    let failed = results.iter().filter(|item| item.status == ManifestStatus::Failed).count();
    assert_eq!(failed, 0);
}

#[test]
fn turtle_runner_enforces_positive_and_negative_expectations() {
    let results = run_manifest_turtle("tests/fixtures/w3c-turtle").expect("run turtle suite");
    assert_eq!(results.len(), 10);
    let failed = results.iter().filter(|item| item.status == ManifestStatus::Failed).count();
    assert_eq!(failed, 0);
}

#[test]
fn trig_runner_enforces_positive_and_negative_expectations() {
    let results = run_manifest_trig("tests/fixtures/w3c-trig").expect("run trig suite");
    assert_eq!(results.len(), 10);
    let failed = results.iter().filter(|item| item.status == ManifestStatus::Failed).count();
    assert_eq!(failed, 0);
}
