use anyhow::Result;
use rdf_engine::{Database, RdfFormat};
use rdf_inference_providers::registry;
use rdf_plugin_api::ReadContext;
use rdf_plugin_inference::{Config, Inference};
use serde_json::{Value, json};
fn config(rules: Vec<&str>) -> Result<Config> {
    Ok(serde_json::from_value(
        json!({"providers":{"local":{"kind":"sparql-rules","config":{}}},"jobs":[{"name":"work","provider":"local","profile":"sparql-construct-once-v1","source_graphs":["default"],"rules":rules}]}),
    )?)
}
fn db(config: Config) -> Result<Database> {
    let mut db = Database::memory()?;
    db.register(Inference::with_registry(config, &registry()?)?)?;
    db.import(
        b"<urn:alice> <urn:worksFor> <urn:acme> .\n<urn:acme> <urn:locatedIn> <urn:france> .",
        RdfFormat::NTriples,
        None,
    )?;
    Ok(db)
}
fn invoke(db: &Database, op: &str) -> Result<Value> {
    db.invoke("inference", op, json!({"job":"work"}))
}
const RULE: &str =
    "CONSTRUCT { ?p <urn:worksIn> ?c } WHERE { ?p <urn:worksFor> ?o . ?o <urn:locatedIn> ?c }";
#[test]
fn one_pass_immutable_snapshot_and_deduplication() -> Result<()> {
    let db = db(config(vec![
        RULE,
        RULE,
        "CONSTRUCT { ?p <urn:secondPass> ?c } WHERE { ?p <urn:worksIn> ?c }",
    ])?)?;
    assert_eq!(invoke(&db, "rebuild")?["triples"], 1);
    assert_eq!(db.query_json("ASK { GRAPH <urn:rdftool:derived:inference:work> { <urn:alice> <urn:worksIn> <urn:france> } }")?["boolean"],true);
    assert_eq!(
        db.query_json(
            "ASK { GRAPH <urn:rdftool:derived:inference:work> { ?s <urn:secondPass> ?o } }"
        )?["boolean"],
        false
    );
    assert_eq!(invoke(&db, "status")?["state"], "ready");
    Ok(())
}
#[test]
fn filters_arithmetic_optional_and_blank_node_joins() -> Result<()> {
    let rule = "CONSTRUCT { ?s <urn:total> ?total } WHERE { ?s <urn:price> ?price; <urn:qty> ?qty . BIND(?price * ?qty AS ?total) FILTER(?total > 10) OPTIONAL { ?s <urn:unused> ?x } }";
    let db = db(config(vec![rule])?)?;
    db.update("INSERT DATA { _:line <urn:price> 7; <urn:qty> 3; <urn:name> 'line' }")?;
    invoke(&db, "rebuild")?;
    assert_eq!(db.query_json("ASK { ?s <urn:name> 'line' . GRAPH <urn:rdftool:derived:inference:work> { ?s <urn:total> 21 } }")?["boolean"],true);
    Ok(())
}
#[test]
fn rejects_network_dataset_clauses_and_non_construct() -> Result<()> {
    for rule in [
        "SELECT * WHERE { ?s ?p ?o }",
        "CONSTRUCT { ?s ?p ?o } FROM <urn:remote> WHERE { ?s ?p ?o }",
        "CONSTRUCT { ?s ?p ?o } WHERE { SERVICE SILENT <http://127.0.0.1:1/> { ?s ?p ?o } }",
        "CONSTRUCT { <urn:a> <urn:b> <urn:c> } WHERE { FILTER EXISTS { SERVICE SILENT <http://127.0.0.1:1/> { ?s ?p ?o } } }",
        "INSERT DATA { <urn:s> <urn:p> <urn:o> }",
    ] {
        let db = db(config(vec![rule])?)?;
        assert!(invoke(&db, "rebuild").is_err(), "{rule}");
        assert_eq!(invoke(&db, "status")?["state"], "missing");
    }
    Ok(())
}
#[test]
fn rule_change_invalidation_failure_preservation_deletion_and_reopen() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut db = Database::open(root.path(), "default", true)?;
    db.register(Inference::with_registry(config(vec![RULE])?, &registry()?)?)?;
    db.update("INSERT DATA { <urn:alice> <urn:worksFor> <urn:acme> .\n<urn:acme> <urn:locatedIn> <urn:france> }")?;
    let original = invoke(&db, "rebuild")?;
    drop(db);
    let mut db = Database::open(root.path(), "default", false)?;
    db.register(Inference::with_registry(config(vec!["malformed rule"])?, &registry()?)?)?;
    assert_eq!(invoke(&db, "status")?["state"], "stale");
    assert!(invoke(&db, "rebuild").is_err());
    assert_eq!(invoke(&db, "status")?["output_digest"], original["output_digest"]);
    drop(db);
    let mut db = Database::open(root.path(), "default", false)?;
    db.register(Inference::with_registry(config(vec![RULE])?, &registry()?)?)?;
    assert_eq!(invoke(&db, "status")?["state"], "ready");
    db.update("DELETE DATA { <urn:alice> <urn:worksFor> <urn:acme> }")?;
    assert_eq!(invoke(&db, "status")?["state"], "stale");
    assert_eq!(invoke(&db, "rebuild")?["triples"], 0);
    Ok(())
}
#[test]
fn output_limit_is_atomic_and_asserted_facts_are_excluded() -> Result<()> {
    let mut c = config(vec!["CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"])?;
    c.providers.get_mut("local").expect("provider").config = json!({"max_triples":1});
    let first = db(c)?;
    assert_eq!(invoke(&first, "rebuild")?["triples"], 0);
    let mut c = config(vec!["CONSTRUCT { ?s <urn:copy> ?o } WHERE { ?s ?p ?o }"])?;
    c.providers.get_mut("local").expect("provider").config = json!({"max_triples":1});
    let other = db(c)?;
    assert!(invoke(&other, "rebuild").is_err());
    assert_eq!(invoke(&other, "status")?["state"], "missing");
    Ok(())
}
