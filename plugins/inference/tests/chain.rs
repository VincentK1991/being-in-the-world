//! Chained jobs: a job reads another job's deductions. A stand-in reasoner keeps this offline.
use anyhow::Result;
use rdf_engine::Database;
use rdf_plugin_api::ReadContext;
use rdf_plugin_inference::{Config, Inference, Reasoner};
use serde_json::{Value, json};
use std::sync::Arc;

/// Dog -> Animal and Animal -> Living, one step each, so two chained jobs are needed for Dog -> Living.
struct Taxonomy;
impl Reasoner for Taxonomy {
    fn identity(&self) -> Result<Value> {
        Ok(json!({"name": "taxonomy", "profiles": ["taxonomy"]}))
    }
    fn infer(&self, _profile: &str, ntriples: &str) -> Result<(String, Value)> {
        let mut out = Vec::new();
        for line in ntriples.lines() {
            let Some((subject, rest)) =
                line.split_once(" <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ")
            else {
                continue;
            };
            let object = rest.trim_end_matches(" .");
            let class = |name: &str| format!("<urn:{name}>");
            if object == class("Dog") {
                out.push(format!(
                    "{subject} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> {} .",
                    class("Animal")
                ));
            } else if object == class("Animal") {
                out.push(format!(
                    "{subject} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> {} .",
                    class("Living")
                ));
            }
        }
        out.sort();
        out.dedup();
        Ok((out.join("\n"), self.identity()?))
    }
}
fn config(jobs: Value) -> Result<Config> {
    Ok(serde_json::from_value(json!({"endpoint": "http://127.0.0.1:1", "jobs": jobs}))?)
}
fn job(name: &str, sources: &[&str]) -> Value {
    json!({"name": name, "profile": "taxonomy", "source_graphs": sources})
}
fn chain() -> Value {
    json!([
        job("animals", &["urn:data"]),
        job("living", &["urn:rdftool:derived:inference:animals"])
    ])
}
fn database() -> Result<Database> {
    let mut db = Database::memory()?;
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Dog> } }")?;
    db.register(Inference::with_reasoner(config(chain())?, Arc::new(Taxonomy))?)?;
    Ok(db)
}
fn run(db: &Database, operation: &str, input: Value) -> Result<Value> {
    db.invoke("inference", operation, input)
}
fn has(db: &Database, graph: &str, subject: &str, class: &str) -> Result<bool> {
    let ask = format!(
        "ASK {{ GRAPH <urn:rdftool:derived:inference:{graph}> {{ <urn:{subject}> a <urn:{class}> }} }}"
    );
    Ok(db.query_json(&ask)?["boolean"] == true)
}

#[test]
fn a_job_reads_another_jobs_output_and_a_cascade_builds_both_in_order() -> Result<()> {
    let db = database()?;
    // Nothing built: the downstream job is waiting on the upstream one.
    let waiting = run(&db, "status", json!({"job": "living"}))?;
    assert_eq!(waiting["state"], "missing");
    assert_eq!(waiting["stale_upstream"], json!(["animals"]));

    let built = run(&db, "rebuild", json!({"job": "living", "cascade": "upstream"}))?;
    assert_eq!(built["state"], "ready");
    assert_eq!(built["cascade"]["steps"], json!([{"job": "animals", "action": "rebuilt"}]));
    assert_eq!(built["ran"], json!([{"job": "animals"}]));
    assert!(has(&db, "animals", "rex", "Animal")?);
    assert!(has(&db, "living", "rex", "Living")?, "the second job saw the first job's deduction");
    assert_eq!(run(&db, "status", json!({"job": "living"}))?["state"], "ready");
    Ok(())
}

#[test]
fn staleness_travels_downstream_and_a_cascade_only_rebuilds_what_changed() -> Result<()> {
    let db = database()?;
    run(&db, "rebuild", json!({"job": "living", "cascade": "upstream"}))?;

    // New data: the upstream job is stale, and so is the downstream one even though its own source
    // graph has not changed yet.
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:fido> a <urn:Dog> } }")?;
    assert_eq!(run(&db, "status", json!({"job": "animals"}))?["state"], "stale");
    let downstream = run(&db, "status", json!({"job": "living"}))?;
    assert_eq!(downstream["state"], "stale");
    assert_eq!(downstream["stale_upstream"], json!(["animals"]));

    let rebuilt = run(&db, "rebuild", json!({"job": "animals", "cascade": "downstream"}))?;
    assert_eq!(rebuilt["cascade"]["steps"], json!([{"job": "living", "action": "rebuilt"}]));
    assert!(has(&db, "living", "fido", "Living")?);

    // Nothing changed this time, so the downstream job is still ready and is skipped.
    let again = run(&db, "rebuild", json!({"job": "animals", "cascade": "downstream"}))?;
    assert_eq!(again["cascade"]["steps"], json!([{"job": "living", "action": "skipped"}]));
    assert_eq!(again["ran"], json!([]));
    Ok(())
}

#[test]
fn lineage_lists_both_directions_and_the_rebuild_order() -> Result<()> {
    let db = database()?;
    let middle = run(&db, "lineage", json!({"job": "animals"}))?;
    assert_eq!(middle["upstream"], json!([]));
    assert_eq!(middle["downstream"][0]["job"], "living");
    assert_eq!(middle["rebuild_order"], json!(["animals", "living"]));
    let last = run(&db, "lineage", json!({"job": "living"}))?;
    assert_eq!(last["upstream"][0]["job"], "animals");
    assert_eq!(last["depends_on"], json!(["animals"]));
    assert_eq!(last["rebuild_order"], json!(["animals", "living"]));
    Ok(())
}

#[test]
fn configurations_that_cannot_be_ordered_or_read_are_rejected() -> Result<()> {
    let reasoner = || Arc::new(Taxonomy) as Arc<dyn Reasoner>;
    let build = |jobs: Value| Inference::with_reasoner(config(jobs)?, reasoner());
    let a = "urn:rdftool:derived:inference:a";
    let b = "urn:rdftool:derived:inference:b";
    let cycle = build(json!([job("a", &[b]), job("b", &[a])])).err().expect("cycle");
    assert!(format!("{cycle:#}").contains("cycle"), "{cycle:#}");
    let itself = build(json!([job("a", &[a])])).err().expect("self reference");
    assert!(format!("{itself:#}").contains("cycle"));
    let unknown =
        build(json!([job("a", &["urn:rdftool:derived:inference:ghost"])])).err().expect("unknown");
    assert!(format!("{unknown:#}").contains("unknown job ghost"));
    let metadata = build(json!([
        job("a", &["urn:data"]),
        job("b", &["urn:rdftool:derived:inference:a:metadata"])
    ]))
    .err()
    .expect("metadata");
    assert!(format!("{metadata:#}").contains("unknown job a:metadata"), "{metadata:#}");
    let foreign =
        build(json!([job("a", &["urn:rdftool:derived:other:x"])])).err().expect("foreign");
    assert!(format!("{foreign:#}").contains("only the output of an inference job"));
    // A longer acyclic chain is fine and is ordered dependencies first.
    let c = build(json!([job("c", &[b]), job("b", &[a]), job("a", &["urn:data"])]));
    assert!(c.is_ok());
    // cascade is only accepted on rebuild, and only with a known value.
    let db = database()?;
    assert!(run(&db, "status", json!({"job": "living", "cascade": "upstream"})).is_err());
    assert!(run(&db, "rebuild", json!({"job": "living", "cascade": "sideways"})).is_err());
    Ok(())
}

#[test]
fn a_failing_upstream_job_stops_the_cascade_and_leaves_downstream_untouched() -> Result<()> {
    struct Broken;
    impl Reasoner for Broken {
        fn identity(&self) -> Result<Value> {
            Taxonomy.identity()
        }
        fn infer(&self, _: &str, _: &str) -> Result<(String, Value)> {
            anyhow::bail!("reasoner exploded")
        }
    }
    let mut db = Database::memory()?;
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:rex> a <urn:Dog> } }")?;
    db.register(Inference::with_reasoner(config(chain())?, Arc::new(Broken))?)?;
    let error =
        run(&db, "rebuild", json!({"job": "living", "cascade": "upstream"})).expect_err("fails");
    assert!(format!("{error:#}").contains("rebuild of job animals failed"), "{error:#}");
    assert_eq!(run(&db, "status", json!({"job": "living"}))?["state"], "missing");
    Ok(())
}
