use rdf_testkit::manifest::{ManifestStatus, run_manifest_ntriples};

#[test]
fn ntriples_runner_enforces_positive_and_negative_expectations() {
    let results = run_manifest_ntriples("tests/fixtures/ntriples").expect("run n-triples suite");
    assert_eq!(results.len(), 6);

    let passed = results.iter().filter(|item| item.status == ManifestStatus::Passed).count();
    let failed = results.iter().filter(|item| item.status == ManifestStatus::Failed).count();

    assert_eq!(passed, 6);
    assert_eq!(failed, 0);
}
