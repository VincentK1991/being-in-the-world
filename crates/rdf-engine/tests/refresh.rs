use anyhow::{Result, bail};
use rdf_engine::{Database, RefreshRule};
use rdf_plugin_api::{Manifest, Plugin, ReadContext, SDK_VERSION};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Records every call; fails when its input asks for it.
struct Recorder {
    id: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Plugin for Recorder {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            version: "0".into(),
            sdk_version: SDK_VERSION,
            operations: vec!["run".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, _: &dyn ReadContext, _: &str, input: Value) -> Result<Value> {
        self.calls.lock().expect("lock").push(format!("{}:{input}", self.id));
        if input["fail"] == true {
            bail!("asked to fail");
        }
        Ok(json!({"ok": true}))
    }
}
fn database(rules: Value) -> Result<(Database, Arc<Mutex<Vec<String>>>)> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut db = Database::memory()?;
    db.register(Recorder { id: "jobs", calls: calls.clone() })?;
    db.register(Recorder { id: "search", calls: calls.clone() })?;
    db.refresh_rules(serde_json::from_value::<Vec<RefreshRule>>(rules)?)?;
    Ok((db, calls))
}
const RULE: &str = r#"[{"after": {"plugin": "jobs", "operation": "run", "input": {"job": "risk"}},
                        "then":  {"plugin": "search", "operation": "run", "input": {"index": "notes"}}}]"#;

#[test]
fn a_matching_operation_runs_its_follow_up_and_reports_it() -> Result<()> {
    let (db, calls) = database(serde_json::from_str(RULE)?)?;
    let result = db.invoke("jobs", "run", json!({"job": "risk"}))?;
    assert_eq!(result["refreshed"][0]["ok"], true);
    assert_eq!(calls.lock().expect("lock").len(), 2);
    // A different job, or the follow-up itself, triggers nothing.
    db.invoke("jobs", "run", json!({"job": "other"}))?;
    db.invoke("search", "run", json!({"index": "notes"}))?;
    assert_eq!(calls.lock().expect("lock").len(), 4);
    Ok(())
}

#[test]
fn a_failing_follow_up_is_reported_without_failing_the_operation() -> Result<()> {
    let rules = r#"[{"after": {"plugin": "jobs", "operation": "run"},
                     "then":  {"plugin": "search", "operation": "run", "input": {"fail": true}}}]"#;
    let (db, _) = database(serde_json::from_str(rules)?)?;
    let result = db.invoke("jobs", "run", json!({}))?;
    assert_eq!(result["ok"], true);
    assert_eq!(result["refreshed"][0]["ok"], false);
    assert!(result["refreshed"][0]["error"].as_str().is_some_and(|e| e.contains("asked to fail")));
    // If the operation itself fails there is nothing to refresh.
    assert!(db.invoke("jobs", "run", json!({"fail": true})).is_err());
    Ok(())
}

#[test]
fn rules_naming_unknown_operations_or_chaining_are_rejected() -> Result<()> {
    let unknown = r#"[{"after": {"plugin": "jobs", "operation": "run"}, "then": {"plugin": "nope", "operation": "run"}}]"#;
    assert!(database(serde_json::from_str(unknown)?).is_err());
    let chain = r#"[{"after": {"plugin": "jobs", "operation": "run"}, "then": {"plugin": "search", "operation": "run"}},
                    {"after": {"plugin": "search", "operation": "run"}, "then": {"plugin": "jobs", "operation": "run"}}]"#;
    assert!(database(serde_json::from_str(chain)?).is_err());
    Ok(())
}
