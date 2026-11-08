//! Rule tests: the `test` operation and the checks made when a configuration loads.
use anyhow::Result;
use rdf_engine::Database;
use rdf_plugin_inference::{Config, Inference, Reasoner};
use serde_json::{Value, json};
use std::sync::Arc;

/// `?x a Dog` becomes `?x a Animal`, and nothing else.
struct Taxonomy;
impl Reasoner for Taxonomy {
    fn identity(&self) -> Result<Value> {
        Ok(json!({"name": "taxonomy", "profiles": ["taxonomy"]}))
    }
    fn infer(&self, _: &str, ntriples: &str) -> Result<(String, Value)> {
        let mut out: Vec<String> = ntriples
            .lines()
            .filter_map(|line| {
                line.strip_suffix(" <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:Dog> .")
            })
            .map(|subject| {
                format!(
                    "{subject} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:Animal> ."
                )
            })
            .collect();
        out.sort();
        Ok((out.join("\n"), self.identity()?))
    }
}
const PREFIXES: &str =
    "@prefix : <urn:> . @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .";
fn config(tests: Value) -> Result<Config> {
    Ok(serde_json::from_value(json!({"endpoint": "http://127.0.0.1:1", "jobs": [
        {"name": "animals", "profile": "taxonomy", "source_graphs": ["urn:data"], "tests": tests}
    ]}))?)
}
fn database(tests: Value) -> Result<Database> {
    let mut db = Database::memory()?;
    db.register(Inference::with_reasoner(config(tests)?, Arc::new(Taxonomy))?)?;
    Ok(db)
}
fn test(name: &str, given: &str, expect: &str, absent: &str, exact: bool) -> Value {
    json!({"name": name, "prefixes": PREFIXES, "given": given, "expect": expect, "absent": absent, "exact": exact})
}

#[test]
fn passing_and_failing_tests_are_reported_with_what_differed() -> Result<()> {
    let db = database(json!([
        test("dog becomes animal", ":rex a :Dog .", ":rex a :Animal .", ":rex a :Cat .", true),
        test("expects too much", ":rex a :Dog .", ":rex a :Animal . :rex a :Pet .", "", false),
        test("forbids what happens", ":rex a :Dog .", "", ":rex a :Animal .", false),
        test("exact catches extras", ":rex a :Dog . :fido a :Dog .", ":rex a :Animal .", "", true),
    ]))?;
    let result = db.invoke("inference", "test", json!({"job": "animals"}))?;
    assert_eq!(result["passed"], false);
    let by_name = |name: &str| {
        result["results"]
            .as_array()
            .expect("results")
            .iter()
            .find(|r| r["name"] == name)
            .expect("test")
            .clone()
    };
    assert_eq!(by_name("dog becomes animal")["passed"], true);
    assert_eq!(
        by_name("expects too much")["missing"],
        json!(["<urn:rex> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:Pet> ."])
    );
    assert_eq!(
        by_name("forbids what happens")["present_but_forbidden"].as_array().expect("list").len(),
        1
    );
    assert_eq!(
        by_name("exact catches extras")["unexpected"],
        json!(["<urn:fido> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:Animal> ."])
    );
    Ok(())
}

#[test]
fn tests_never_touch_the_store_and_all_jobs_can_be_run_at_once() -> Result<()> {
    let db = database(json!([test("ok", ":rex a :Dog .", ":rex a :Animal .", "", false)]))?;
    let before = db.overview()?.quads;
    let all = db.invoke("inference", "test", json!({}))?;
    assert_eq!(all["passed"], true);
    assert_eq!(all["jobs"][0]["job"], "animals");
    assert_eq!(db.overview()?.quads, before, "no derived graph was written");
    assert_eq!(db.invoke("inference", "status", json!({"job": "animals"}))?["state"], "missing");
    Ok(())
}

#[test]
fn bad_tests_are_rejected_when_the_configuration_loads() {
    let load = |tests: Value| -> String {
        match config(tests).and_then(|c| Inference::with_reasoner(c, Arc::new(Taxonomy))) {
            Ok(_) => String::new(),
            Err(error) => format!("{error:#}"),
        }
    };
    assert!(load(json!([test("a", ":x a :Dog .", ":x a :Animal .", "", false)])).is_empty());
    assert!(
        load(json!([test("bad turtle", ":x a", ":x a :Animal .", "", false)]))
            .contains("not valid Turtle")
    );
    assert!(
        load(json!([test("blank", ":x a :Dog .", "[] a :Animal .", "", false)]))
            .contains("blank node")
    );
    assert!(
        load(json!([test("nothing", ":x a :Dog .", "", "", false)])).contains("checks nothing")
    );
    assert!(
        load(json!([
            test("a", ":x a :Dog .", ":x a :Animal .", "", false),
            test("a", ":x a :Dog .", ":x a :Animal .", "", false)
        ]))
        .contains("unique")
    );
    assert!(
        load(json!([{"name": "x", "given": ":x a :Dog .", "unknown": 1}]))
            .contains("unknown field")
    );
}

#[test]
fn editing_a_test_does_not_change_what_a_job_looks_like_to_the_staleness_check() -> Result<()> {
    let one = serde_json::to_value(config(json!([test(
        "a",
        ":x a :Dog .",
        ":x a :Animal .",
        "",
        false
    )]))?)?;
    let other = serde_json::to_value(config(json!([test(
        "b",
        ":y a :Dog .",
        ":y a :Animal .",
        "",
        true
    )]))?)?;
    assert_eq!(
        one, other,
        "tests are not part of the serialized job, so they do not feed its fingerprint"
    );
    Ok(())
}
