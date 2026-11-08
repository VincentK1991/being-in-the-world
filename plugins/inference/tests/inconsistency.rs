//! An inconsistent input is a result: the job keeps its last output, reads as `inconsistent` with
//! the clashes, blocks jobs that read it, and recovers when the data is fixed.
use anyhow::Result;
use rdf_engine::Database;
use rdf_plugin_api::ReadContext;
use rdf_plugin_inference::{Config, Inconsistent, Inference, Reasoner};
use serde_json::{Value, json};
use std::sync::Arc;

const TYPE: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
/// Dog -> Animal; a thing typed both Dog and Cat is a clash.
struct Strict;
impl Reasoner for Strict {
    fn identity(&self) -> Result<Value> {
        Ok(json!({"name": "strict", "profiles": ["strict"]}))
    }
    fn infer(&self, _profile: &str, ntriples: &str) -> Result<(String, Value)> {
        let typed = |class: &str| -> Vec<&str> {
            ntriples
                .lines()
                .filter(|l| l.contains(TYPE) && l.contains(&format!("<urn:{class}>")))
                .filter_map(|l| l.split(' ').next())
                .collect()
        };
        let cats = typed("Cat");
        let both: Vec<&str> = typed("Dog").into_iter().filter(|d| cats.contains(d)).collect();
        if let Some(subject) = both.first() {
            return Err(Inconsistent {
                clashes: vec![json!({"message": format!("{subject} is both a Dog and a Cat"),
                                     "terms": [subject.trim_matches(['<', '>'])]})],
            }
            .into());
        }
        let out: Vec<String> =
            typed("Dog").iter().map(|s| format!("{s} {TYPE} <urn:Animal> .")).collect();
        Ok((out.join("\n"), self.identity()?))
    }
}
fn config() -> Result<Config> {
    Ok(serde_json::from_value(json!({"endpoint": "http://127.0.0.1:1", "jobs": [
        {"name": "animals", "profile": "strict", "source_graphs": ["urn:data"],
         "tests": [{"name": "dog and cat clash", "given": "<urn:a> a <urn:Dog>, <urn:Cat> .", "inconsistent": true},
                   {"name": "a dog is fine", "given": "<urn:a> a <urn:Dog> .", "inconsistent": true}]},
        {"name": "downstream", "profile": "strict", "source_graphs": ["urn:rdftool:derived:inference:animals"]}
    ]}))?)
}
fn database() -> Result<Database> {
    let config = config()?;
    let mut db = Database::memory()?;
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Dog> } }")?;
    db.register(Inference::with_reasoner(config, Arc::new(Strict))?)?;
    Ok(db)
}
fn run(db: &Database, operation: &str, input: Value) -> Result<Value> {
    db.invoke("inference", operation, input)
}

#[test]
fn an_inconsistent_rebuild_keeps_the_last_output_and_reports_the_clash() -> Result<()> {
    let db = database()?;
    assert_eq!(run(&db, "rebuild", json!({"job": "animals"}))?["state"], "ready");
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Cat> } }")?;

    let rebuilt = run(&db, "rebuild", json!({"job": "animals"}))?;
    assert_eq!(rebuilt["state"], "inconsistent");
    assert_eq!(rebuilt["kept"], "stale");
    assert_eq!(rebuilt["clashes"][0]["terms"], json!(["urn:rex"]));
    let kept = "ASK { GRAPH <urn:rdftool:derived:inference:animals> { <urn:rex> a <urn:Animal> } }";
    assert_eq!(db.query_json(kept)?["boolean"], true, "the last good output stays published");

    let status = run(&db, "status", json!({"job": "animals"}))?;
    assert_eq!(status["state"], "inconsistent");
    assert!(status["clashes"][0]["message"].as_str().is_some_and(|m| m.contains("Dog and a Cat")));

    // Fixing the data turns it back into an ordinary stale job, and a rebuild makes it ready.
    db.update("DELETE DATA { GRAPH <urn:data> { <urn:rex> a <urn:Cat> } }")?;
    assert_ne!(run(&db, "status", json!({"job": "animals"}))?["state"], "inconsistent");
    assert_eq!(run(&db, "rebuild", json!({"job": "animals"}))?["state"], "ready");
    assert!(run(&db, "status", json!({"job": "animals"}))?.get("clashes").is_none());
    Ok(())
}

#[test]
fn a_cascade_stops_at_an_inconsistent_job() -> Result<()> {
    let db = database()?;
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Cat> } }")?;
    let result = run(&db, "rebuild", json!({"job": "downstream", "cascade": "upstream"}))?;
    assert_eq!(result["state"], "blocked");
    assert_eq!(result["blocked_by"], "animals");
    assert_eq!(result["cascade"]["steps"], json!([{"job": "animals", "action": "inconsistent"}]));
    assert_eq!(result["ran"], json!([]), "nothing downstream ran");

    let forward = run(&db, "rebuild", json!({"job": "animals", "cascade": "downstream"}))?;
    assert_eq!(forward["state"], "inconsistent");
    assert_eq!(
        forward["cascade"]["steps"],
        json!([{"job": "downstream", "action": "blocked", "blocked_by": "animals"}])
    );
    Ok(())
}

#[test]
fn a_rule_test_can_expect_an_inconsistency() -> Result<()> {
    let db = database()?;
    let tests = run(&db, "test", json!({"job": "animals"}))?;
    let results = tests["results"].as_array().cloned().unwrap_or_default();
    assert_eq!(results[0]["passed"], true);
    assert_eq!(results[0]["inconsistent"], true);
    assert_eq!(results[1]["passed"], false, "consistent data fails an inconsistency test");
    Ok(())
}

#[test]
fn the_clash_record_survives_a_restart_and_a_successful_rebuild_clears_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    {
        let mut db = Database::open(dir.path(), "default", true)?;
        db.register(Inference::with_reasoner(config()?, Arc::new(Strict))?)?;
        db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Dog> , <urn:Cat> } }")?;
        assert_eq!(run(&db, "rebuild", json!({"job": "animals"}))?["state"], "inconsistent");
    }
    let mut reopened = Database::open(dir.path(), "default", false)?;
    reopened.register(Inference::with_reasoner(config()?, Arc::new(Strict))?)?;
    let status = run(&reopened, "status", json!({"job": "animals"}))?;
    assert_eq!(status["state"], "inconsistent", "{status}");
    assert!(status.get("attempt").is_none(), "the raw record is not exposed");
    reopened.update("DELETE DATA { GRAPH <urn:data> { <urn:rex> a <urn:Cat> } }")?;
    assert_eq!(run(&reopened, "rebuild", json!({"job": "animals"}))?["state"], "ready");
    let meta = "ASK { GRAPH <urn:rdftool:derived:inference:animals:metadata> { ?s <urn:rdftool:attempt> ?o } }";
    assert_eq!(reopened.query_json(meta)?["boolean"], false, "publishing cleared the record");
    Ok(())
}
