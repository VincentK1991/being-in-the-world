use anyhow::Result;
use rdf_engine::{Database, RdfFormat};
use rdf_plugin_api::ReadContext;
use rdf_plugin_inference::{Config, Inference};
use serde_json::json;
fn config() -> Result<Config> {
    Ok(serde_json::from_str(include_str!("../example-config.json"))?)
}
#[test]
#[ignore = "requires OWL-RL sidecar on 127.0.0.1:18082"]
fn real_owlrl_deductions_rebuild_failure_isolation_and_reopen() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut db = Database::open(dir.path(), "default", true)?;
    db.register(Inference::new(config()?)?)?;
    db.import(include_bytes!("../example.trig"), RdfFormat::TriG, None)?;
    let invoke = |db: &Database, operation: &str| {
        db.invoke("inference", operation, json!({"job":"vehicles"}))
    };
    assert_eq!(invoke(&db, "status")?["state"], "missing");
    let build = invoke(&db, "rebuild")?;
    assert_eq!(build["state"], "ready");
    for triple in [
        "<https://example.org/car1> a <https://example.org/Vehicle>",
        "<https://example.org/car1> a <https://example.org/Asset>",
        "<https://example.org/bob> <https://example.org/hasChild> <https://example.org/alice>",
        "<https://example.org/office> <https://example.org/locatedIn> <https://example.org/country>",
    ] {
        assert_eq!(
            db.query_json(&format!(
                "ASK {{ GRAPH <urn:rdftool:derived:inference:vehicles> {{ {triple} }} }}"
            ))?["boolean"],
            true,
            "{triple}"
        );
    }
    assert_eq!(db.query_json("ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/secret> a <https://example.org/Vehicle> } }")?["boolean"],false);
    // Blank-node identity must survive the sidecar and join to asserted RDF.
    db.update("INSERT DATA { GRAPH <urn:example:data> { _:x a <https://example.org/ElectricCar>; <urn:name> 'anonymous car' } }")?;
    invoke(&db, "rebuild")?;
    assert_eq!(db.query_json("ASK { GRAPH <urn:example:data> { ?s <urn:name> 'anonymous car' } GRAPH <urn:rdftool:derived:inference:vehicles> { ?s a <https://example.org/Vehicle> } }")?["boolean"],true);
    drop(db);
    let mut db = Database::open(dir.path(), "default", false)?;
    db.register(Inference::new(config()?)?)?;
    assert_eq!(invoke(&db, "status")?["state"], "ready");
    db.update("DELETE DATA { GRAPH <urn:example:data> { <https://example.org/car1> a <https://example.org/ElectricCar> } }")?;
    assert_eq!(invoke(&db, "status")?["state"], "stale");
    invoke(&db, "rebuild")?;
    assert_eq!(db.query_json("ASK { GRAPH <urn:rdftool:derived:inference:vehicles> { <https://example.org/car1> a <https://example.org/Vehicle> } }")?["boolean"],false);
    let previous = invoke(&db, "status")?["output_digest"].clone();
    db.update("INSERT DATA { GRAPH <urn:example:ontology> { <urn:unsupported> <http://www.w3.org/2002/07/owl#unionOf> <urn:list> } }")?;
    assert!(invoke(&db, "rebuild").is_err());
    assert_eq!(invoke(&db, "status")?["output_digest"], previous);
    let mut other = Database::open(dir.path(), "other", true)?;
    other.register(Inference::new(config()?)?)?;
    assert_eq!(invoke(&other, "status")?["state"], "missing");
    Ok(())
}
