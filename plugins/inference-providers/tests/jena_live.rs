use anyhow::Result;
use rdf_engine::{Database, RdfFormat};
use rdf_inference_providers::registry;
use rdf_plugin_api::ReadContext;
use rdf_plugin_inference::{Config, Inference};
use serde_json::{Value, json};
fn config() -> Result<Config> {
    Ok(serde_json::from_str(include_str!("../../inference/example-rules-config.json"))?)
}
fn invoke(db: &Database, job: &str, op: &str) -> Result<Value> {
    db.invoke("inference", op, json!({"job":job}))
}
#[test]
#[ignore = "requires real Jena sidecar on 127.0.0.1:18083"]
fn jena_and_sparql_providers_parity_chaining_staleness_and_recovery() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut db = Database::open(dir.path(), "default", true)?;
    db.register(Inference::with_registry(config()?, &registry()?)?)?;
    db.import(include_bytes!("../../inference/examples/rules/data.trig"), RdfFormat::TriG, None)?;
    for job in ["work-jena", "work-sparql", "totals"] {
        assert_eq!(invoke(&db, job, "rebuild")?["state"], "ready");
    }
    for job in ["work-jena", "work-sparql"] {
        assert_eq!(db.query_json(&format!("ASK {{ GRAPH <urn:rdftool:derived:inference:{job}> {{ <https://example.org/alice> <https://example.org/worksInCountry> <https://example.org/France> }} }}"))?["boolean"],true);
        assert_eq!(db.query_json(&format!("ASK {{ GRAPH <urn:rdftool:derived:inference:{job}> {{ <https://example.org/secret> ?p ?o }} }}"))?["boolean"],false);
    }
    for (job, expected) in [("work-jena", true), ("work-sparql", false)] {
        assert_eq!(db.query_json(&format!("ASK {{ GRAPH <urn:rdftool:derived:inference:{job}> {{ <https://example.org/alice> <https://example.org/worksOnContinent> <https://example.org/Europe> }} }}"))?["boolean"],expected);
    }
    db.update("INSERT DATA { GRAPH <urn:rules:data> { _:person <https://example.org/worksFor> <https://example.org/acme>; <urn:name> 'anonymous' } }")?;
    invoke(&db, "work-jena", "rebuild")?;
    assert_eq!(db.query_json("ASK { GRAPH <urn:rules:data> { ?s <urn:name> 'anonymous' } GRAPH <urn:rdftool:derived:inference:work-jena> { ?s <https://example.org/worksInCountry> <https://example.org/France> } }")?["boolean"],true);
    let previous = invoke(&db, "work-jena", "status")?["output_digest"].clone();
    drop(db);
    let mut changed = config()?;
    changed.jobs.iter_mut().find(|j| j.name == "work-jena").expect("job").rules =
        vec!["[bad: (?s <urn:p> ?o) -> (?unbound <urn:q> ?o)]".into()];
    let mut db = Database::open(dir.path(), "default", false)?;
    db.register(Inference::with_registry(changed, &registry()?)?)?;
    assert_eq!(invoke(&db, "work-jena", "status")?["state"], "stale");
    assert!(invoke(&db, "work-jena", "rebuild").is_err());
    assert_eq!(invoke(&db, "work-jena", "status")?["output_digest"], previous);
    drop(db);
    let mut db = Database::open(dir.path(), "default", false)?;
    db.register(Inference::with_registry(config()?, &registry()?)?)?;
    assert_eq!(invoke(&db, "work-jena", "status")?["state"], "ready");
    db.update("DELETE DATA { GRAPH <urn:rules:data> { <https://example.org/alice> <https://example.org/worksFor> <https://example.org/acme> } }")?;
    assert_eq!(invoke(&db, "work-jena", "status")?["state"], "stale");
    invoke(&db, "work-jena", "rebuild")?;
    assert_eq!(db.query_json("ASK { GRAPH <urn:rdftool:derived:inference:work-jena> { <https://example.org/alice> ?p ?o } }")?["boolean"],false);
    Ok(())
}
