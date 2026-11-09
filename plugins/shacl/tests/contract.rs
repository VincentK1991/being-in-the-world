use anyhow::{Result, bail};
use rdf_engine::{Database, NamedNode, RdfFormat};
use rdf_plugin_api::ReadContext;
use rdf_plugin_shacl::{Config, Finding, Job, Report, Severity, Shacl, Validator};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn config(threshold: Severity) -> Config {
    Config {
        endpoint: "http://127.0.0.1:18083".into(),
        jobs: vec![Job {
            name: "test".into(),
            data_graphs: vec!["urn:data".into()],
            shapes_graphs: vec!["urn:shapes".into()],
            fail_on: threshold,
        }],
    }
}
struct Fake {
    severity: &'static str,
    fail: Arc<AtomicBool>,
    mutate: Option<Database>,
}
impl Validator for Fake {
    fn validate(&self, data: &str, shapes: &str) -> Result<Report> {
        assert!(data.contains("urn:person"));
        assert!(!data.contains("urn:private"));
        assert!(shapes.contains("urn:Shape"));
        if self.fail.load(Ordering::SeqCst) {
            bail!("provider unavailable");
        }
        if let Some(db) = &self.mutate {
            db.update("INSERT DATA { GRAPH <urn:data> { <urn:new> <urn:p> 1 } }")?;
        }
        Ok(Report {
            conforms: false,
            engine_version: "6.2.0".into(),
            ntriples: "<urn:report> <http://www.w3.org/ns/shacl#conforms> \"false\"^^<http://www.w3.org/2001/XMLSchema#boolean> .".into(),
            findings: vec![Finding {
                severity: self.severity.into(),
                focus_node: vec!["<urn:person>".into()],
                result_path: vec![],
                value: vec![],
                source_shape: vec![],
                constraint: vec![],
                messages: vec![],
            }],
        })
    }
}
fn seed(db: &Database) -> Result<()> {
    db.import(b"<urn:person> <urn:p> 1 .", RdfFormat::Turtle, Some(NamedNode::new("urn:data")?))?;
    db.import(b"<urn:Shape> <urn:p> 1 .", RdfFormat::Turtle, Some(NamedNode::new("urn:shapes")?))?;
    db.update("INSERT DATA { GRAPH <urn:private> { <urn:private> <urn:p> 1 } }")
}
fn call(db: &Database, op: &str) -> Result<Value> {
    db.invoke("shacl", op, json!({"job":"test"}))
}
fn register(
    db: &mut Database,
    severity: &'static str,
    threshold: Severity,
) -> Result<Arc<AtomicBool>> {
    let fail = Arc::new(AtomicBool::new(false));
    db.register(Shacl::with_validator(
        config(threshold),
        Arc::new(Fake { severity, fail: fail.clone(), mutate: None }),
    )?)?;
    Ok(fail)
}
#[test]
fn report_only_keeps_data_and_gate_failure_preserves_report() -> Result<()> {
    let mut db = Database::memory()?;
    seed(&db)?;
    let fail = register(&mut db, "Violation", Severity::Violation)?;
    let report = call(&db, "validate")?;
    assert_eq!(report["summary"]["gate_passed"], false);
    assert_eq!(report["mode"], "report-only");
    assert_eq!(call(&db, "status")?["state"], "ready");
    assert!(call(&db, "check").is_err());
    assert_eq!(call(&db, "status")?["state"], "ready");
    assert_eq!(
        db.query_json("ASK { GRAPH <urn:data> { <urn:person> <urn:p> 1 } }")?["boolean"],
        true
    );
    fail.store(true, Ordering::SeqCst);
    assert!(call(&db, "validate").is_err());
    assert_eq!(call(&db, "status")?["state"], "ready");
    db.update("INSERT DATA { GRAPH <urn:shapes> { <urn:Shape> <urn:p> 2 } }")?;
    assert_eq!(call(&db, "status")?["state"], "stale");
    Ok(())
}
#[test]
fn thresholds_are_separate_from_shacl_conformance() -> Result<()> {
    for (severity, threshold, passes) in [
        ("Warning", Severity::Violation, true),
        ("Info", Severity::Warning, true),
        ("Warning", Severity::Warning, false),
        ("Info", Severity::Info, false),
        ("Violation", Severity::Info, false),
    ] {
        let mut db = Database::memory()?;
        seed(&db)?;
        register(&mut db, severity, threshold)?;
        let report = call(&db, "validate")?;
        assert_eq!(report["summary"]["conforms"], false);
        assert_eq!(report["summary"]["gate_passed"], passes);
        assert_eq!(call(&db, "check").is_ok(), passes);
    }
    Ok(())
}
#[test]
fn concurrent_source_change_does_not_publish() -> Result<()> {
    let mut db = Database::memory()?;
    seed(&db)?;
    db.register(Shacl::with_validator(
        config(Severity::Violation),
        Arc::new(Fake {
            severity: "Violation",
            fail: Arc::new(AtomicBool::new(false)),
            mutate: Some(db.clone()),
        }),
    )?)?;
    assert!(call(&db, "validate").is_err());
    assert_eq!(call(&db, "status")?["state"], "missing");
    Ok(())
}
#[test]
fn report_survives_reopen_and_is_dataset_scoped() -> Result<()> {
    let root = tempfile::tempdir()?;
    {
        let mut db = Database::open(root.path(), "a", true)?;
        seed(&db)?;
        register(&mut db, "Warning", Severity::Violation)?;
        call(&db, "validate")?;
    }
    let mut a = Database::open(root.path(), "a", false)?;
    register(&mut a, "Warning", Severity::Violation)?;
    assert_eq!(call(&a, "status")?["state"], "ready");
    let mut b = Database::open(root.path(), "b", true)?;
    register(&mut b, "Warning", Severity::Violation)?;
    assert_eq!(call(&b, "status")?["state"], "missing");
    assert!(call(&b, "validate").is_err()); // Empty shapes must not appear to pass.
    a.update("CLEAR GRAPH <urn:rdftool:derived:shacl:test>")?;
    assert_eq!(call(&a, "status")?["state"], "stale");
    Ok(())
}
#[test]
fn configuration_and_inputs_fail_explicitly() -> Result<()> {
    for endpoint in [
        "http://example.org",
        "http://localhost:18083",
        "http://127.0.0.1/a",
        "http://user@127.0.0.1",
    ] {
        let mut cfg = config(Severity::Violation);
        cfg.endpoint = endpoint.into();
        assert!(Shacl::new(cfg).is_err());
    }
    let mut cfg = config(Severity::Violation);
    cfg.jobs[0].shapes_graphs = vec!["urn:data".into()];
    assert!(Shacl::new(cfg).is_err());
    let mut db = Database::memory()?;
    register(&mut db, "Warning", Severity::Violation)?;
    assert!(db.invoke("shacl", "validate", json!({"job":"missing"})).is_err());
    assert!(db.invoke("shacl", "validate", json!({"job":"test","unknown":1})).is_err());
    assert_eq!(db.invoke("shacl", "jobs", json!({}))?["automatic_write_enforcement"], false);
    Ok(())
}
