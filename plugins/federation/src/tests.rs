use super::*;

fn federation(allow: &[&str], local: Option<&str>) -> Result<Federation> {
    Federation::new(FederationConfig {
        endpoint: "http://127.0.0.1:18095/sparql".into(),
        local_source: local.map(str::to_owned),
        allow: allow.iter().map(|s| (*s).to_owned()).collect(),
        timeout_secs: 5,
    })
}
#[test]
fn finds_service_targets_everywhere_a_query_can_hide_them() -> Result<()> {
    let nested = "SELECT * WHERE { ?s ?p ?o . OPTIONAL { SERVICE SILENT <http://b/q> { ?s ?p ?x } } FILTER EXISTS { SERVICE <http://c/q> { ?s ?p ?y } } { SELECT ?s WHERE { SERVICE <http://d/q> { ?s ?p ?z } } } }";
    assert_eq!(service_targets(nested)?, ["http://b/q", "http://c/q", "http://d/q"]);
    assert!(service_targets("SELECT * WHERE { ?s ?p ?o }")?.is_empty());
    assert!(service_targets("SELECT * WHERE { SERVICE ?e { ?s ?p ?o } }").is_err());
    assert!(service_targets("SELECT broken").is_err());
    assert!(
        service_targets("INSERT DATA { <a:a> <a:b> <a:c> }").is_err(),
        "updates are not queries"
    );
    Ok(())
}
#[test]
fn federation_refuses_endpoints_outside_the_allowlist_before_any_request() -> Result<()> {
    let plugin = federation(&["http://ok/q"], Some("http://local/q"))?;
    let error = plugin
        .authorize("SELECT * WHERE { SERVICE <http://evil/q> { ?s ?p ?o } }")
        .expect_err("denied");
    assert!(error.to_string().contains("not registered"), "{error}");
    assert!(plugin.authorize("SELECT * WHERE { { SERVICE <http://ok/q> { ?s ?p ?o } } UNION { SERVICE <http://evil/q> { ?s ?p ?o } } }").is_err());
    assert_eq!(
        plugin.authorize("SELECT * WHERE { SERVICE <http://ok/q> { ?s ?p ?o } }")?,
        ["http://ok/q"]
    );
    assert!(
        federation(&["http://ok/q"], None)?.authorize("SELECT * WHERE { ?s ?p ?o }").is_err(),
        "no SERVICE and no local source"
    );
    Ok(())
}
#[test]
fn configuration_is_validated() {
    assert!(federation(&[], None).is_err());
    assert!(federation(&["ftp://x/q"], None).is_err());
    assert!(federation(&["not an iri"], None).is_err());
    assert!(
        Federation::new(FederationConfig {
            endpoint: "http://example.org/sparql".into(),
            local_source: None,
            allow: vec!["http://ok/q".into()],
            timeout_secs: 5
        })
        .is_err()
    );
    assert!(
        Federation::new(FederationConfig {
            endpoint: "http://127.0.0.1:1/sparql".into(),
            local_source: None,
            allow: vec!["http://ok/q".into()],
            timeout_secs: 0
        })
        .is_err()
    );
    assert!(
        VirtualGraph::new(VirtualGraphConfig {
            name: "x".into(),
            endpoint: "https://127.0.0.1:1/sparql".into(),
            description: None,
            timeout_secs: 5
        })
        .is_err()
    );
    assert!(
        serde_json::from_str::<VirtualGraphConfig>(
            r#"{"name":"a","endpoint":"http://127.0.0.1:1","extra":1}"#
        )
        .is_err()
    );
}
