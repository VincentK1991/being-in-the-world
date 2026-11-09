use anyhow::Result;
use rdf_engine::{Database, NamedNode, RdfFormat};
use rdf_plugin_api::ReadContext;
use rdf_plugin_shacl::{Config, Job, Severity, Shacl};
use serde_json::json;

fn plugin() -> Result<Shacl> {
    Shacl::new(Config {
        endpoint: "http://127.0.0.1:18083".into(),
        jobs: vec![Job {
            name: "claims".into(),
            data_graphs: vec!["urn:data".into()],
            shapes_graphs: vec!["urn:shapes".into()],
            fail_on: Severity::Violation,
        }],
    })
}
#[test]
#[ignore = "requires real Jena sidecar: just shacl"]
fn jena_core_reports_gates_recovery_paths_and_isolation() -> Result<()> {
    let mut db = Database::memory()?;
    db.register(plugin()?)?;
    let graph = |iri| NamedNode::new(iri).map(Some);
    db.import(include_bytes!("../examples/shapes.ttl"), RdfFormat::Turtle, graph("urn:shapes")?)?;
    db.import(include_bytes!("../examples/invalid.ttl"), RdfFormat::Turtle, graph("urn:data")?)?;
    db.import(
        include_bytes!("../examples/valid.ttl"),
        RdfFormat::Turtle,
        graph("urn:unselected")?,
    )?;
    let input = json!({"job":"claims"});
    let report = db.invoke("shacl", "validate", input.clone())?;
    assert_eq!(report["summary"]["counts"], json!({"violation":2,"warning":1,"info":1}));
    assert_eq!(report["summary"]["conforms"], false);
    assert!(db.invoke("shacl", "check", input.clone()).is_err());
    assert_eq!(db.query_json("PREFIX sh: <http://www.w3.org/ns/shacl#> ASK { GRAPH <urn:rdftool:derived:shacl:claims> { ?r a sh:ValidationReport; sh:result ?finding . ?finding sh:focusNode <urn:shacl:example:claim1> } }")?["boolean"],true);
    db.update("DELETE DATA { GRAPH <urn:data> { <urn:shacl:example:claim1> <urn:shacl:example:reserve> -10 } }; INSERT DATA { GRAPH <urn:data> { <urn:shacl:example:claim1> <urn:shacl:example:reserve> 100; <urn:shacl:example:policy> <urn:shacl:example:policy1> } }")?;
    assert_eq!(db.invoke("shacl", "status", input.clone())?["state"], "stale");
    let warning = db.invoke("shacl", "check", input.clone())?;
    assert_eq!(warning["summary"]["conforms"], false);
    assert_eq!(warning["summary"]["gate_passed"], true);
    db.update("CLEAR GRAPH <urn:data>")?;
    db.import(include_bytes!("../examples/valid.ttl"), RdfFormat::Turtle, graph("urn:data")?)?;
    assert_eq!(db.invoke("shacl", "check", input.clone())?["summary"]["conforms"], true);
    // Malformed/unsupported shape changes fail and preserve the previous complete report.
    db.update("INSERT DATA { GRAPH <urn:shapes> { <urn:bad> <http://www.w3.org/ns/shacl#sparql> <urn:query> } }")?;
    assert!(db.invoke("shacl", "validate", input.clone()).is_err());
    assert_eq!(db.invoke("shacl", "status", input)?["metadata"]["conforms"], true);
    // Standard Core property paths and blank-node focus nodes.
    let mut paths = Database::memory()?;
    paths.register(plugin()?)?;
    paths.import(b"@prefix sh: <http://www.w3.org/ns/shacl#> . <urn:Shape> a sh:NodeShape; sh:targetClass <urn:Person>; sh:property [ sh:path (<urn:worksFor> <urn:name>); sh:minCount 1 ] .", RdfFormat::Turtle,graph("urn:shapes")?)?;
    paths.import(
        b"[] a <urn:Person>; <urn:worksFor> <urn:company> .",
        RdfFormat::Turtle,
        graph("urn:data")?,
    )?;
    let report = paths.invoke("shacl", "validate", json!({"job":"claims"}))?;
    assert_eq!(report["summary"]["counts"]["violation"], 1);
    assert!(report["findings"][0]["focusNode"][0].as_str().is_some_and(|s| s.starts_with("_:")));
    assert_eq!(paths.query_json("PREFIX sh: <http://www.w3.org/ns/shacl#> ASK { GRAPH <urn:data> { ?person a <urn:Person> } GRAPH <urn:rdftool:derived:shacl:claims> { ?finding sh:focusNode ?person } }")?["boolean"],true);
    Ok(())
}
