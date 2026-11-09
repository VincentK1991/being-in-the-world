//! Explanations must apply the same freshness policy as job status.
use anyhow::Result;
use rdf_engine::Database;
use rdf_plugin_inference::{Config, Inference, Reasoner};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const TARGET: &str = "<urn:result> <urn:p> <urn:object> .";
struct Explaining {
    revision: Arc<AtomicUsize>,
}
impl Reasoner for Explaining {
    fn identity(&self) -> Result<Value> {
        Ok(json!({"profiles":["test"],"revision":self.revision.load(Ordering::SeqCst)}))
    }
    fn infer(&self, _: &str, _: &str) -> Result<(String, Value)> {
        Ok((TARGET.into(), self.identity()?))
    }
    fn explain(&self, _: &str, _: &str, _: &[String], _: &str) -> Result<Option<Vec<Value>>> {
        Ok(Some(vec![json!({"rule":"test", "premises":[]})]))
    }
}
fn config(endpoint: &str, jobs: Value) -> Result<Config> {
    Ok(serde_json::from_value(json!({"endpoint":endpoint,"jobs":jobs}))?)
}
fn job(name: &str, sources: &[&str]) -> Value {
    json!({"name":name,"profile":"test","source_graphs":sources})
}
fn invoke(db: &Database, operation: &str, name: &str) -> Result<Value> {
    db.invoke(
        "inference",
        operation,
        if operation == "explain" {
            json!({"job":name,"triple":TARGET})
        } else {
            json!({"job":name})
        },
    )
}
fn assert_stale(db: &Database, name: &str) -> Result<()> {
    assert_eq!(invoke(db, "status", name)?["state"], "stale");
    let explanation = invoke(db, "explain", name)?;
    assert_eq!(explanation["explanations"][0]["state"], "stale");
    assert_eq!(explanation["explanations"][0]["stale"], true);
    Ok(())
}

#[test]
fn explanations_notice_provider_identity_changes() -> Result<()> {
    let revision = Arc::new(AtomicUsize::new(1));
    let mut db = Database::memory()?;
    db.register(Inference::with_reasoner(
        config("http://127.0.0.1:1", json!([job("test", &["urn:data"])]))?,
        Arc::new(Explaining { revision: revision.clone() }),
    )?)?;
    invoke(&db, "rebuild", "test")?;
    assert_eq!(invoke(&db, "explain", "test")?["explanations"][0]["stale"], false);
    revision.store(2, Ordering::SeqCst);
    assert_stale(&db, "test")
}

#[test]
fn explanations_notice_configuration_changes_after_reopen() -> Result<()> {
    let root = tempfile::tempdir()?;
    let jobs = json!([job("test", &["urn:data"])]);
    {
        let mut db = Database::open(root.path(), "default", true)?;
        db.register(Inference::with_reasoner(
            config("http://127.0.0.1:1", jobs.clone())?,
            Arc::new(Explaining { revision: Arc::new(AtomicUsize::new(1)) }),
        )?)?;
        invoke(&db, "rebuild", "test")?;
    }
    let mut reopened = Database::open(root.path(), "default", false)?;
    reopened.register(Inference::with_reasoner(
        config("http://127.0.0.1:2", jobs)?,
        Arc::new(Explaining { revision: Arc::new(AtomicUsize::new(1)) }),
    )?)?;
    assert_stale(&reopened, "test")
}

#[test]
fn explanations_notice_upstream_staleness_without_changed_output() -> Result<()> {
    let mut db = Database::memory()?;
    db.register(Inference::with_reasoner(
        config(
            "http://127.0.0.1:1",
            json!([job("up", &["urn:data"]), job("down", &["urn:rdftool:derived:inference:up"])]),
        )?,
        Arc::new(Explaining { revision: Arc::new(AtomicUsize::new(1)) }),
    )?)?;
    db.invoke("inference", "rebuild", json!({"job":"down","cascade":"upstream"}))?;
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:new> <urn:p> 1 } }")?;
    assert_stale(&db, "down")
}

#[test]
fn explanations_notice_changed_outside_inputs() -> Result<()> {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Mutex, atomic::AtomicBool},
        time::Duration,
    };
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let body = Arc::new(Mutex::new("<urn:external> <urn:p> 1 .".to_owned()));
    let stopped = Arc::new(AtomicBool::new(false));
    let (body_copy, stop_copy) = (body.clone(), stopped.clone());
    let server = std::thread::spawn(move || {
        while !stop_copy.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2))).expect("timeout");
                    let mut buffer = [0; 8192];
                    let _ = stream.read(&mut buffer);
                    let text = body_copy.lock().expect("body").clone();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/turtle\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("listener: {e}"),
            }
        }
    });
    let result = (|| -> Result<()> {
        let mut entry = job("test", &["urn:data"]);
        entry["inputs"] = json!([{"name":"outside","kind":"sparql","endpoint":format!("http://{address}/sparql"),"query":"CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"}]);
        let mut db = Database::memory()?;
        db.register(Inference::with_reasoner(
            config("http://127.0.0.1:1", json!([entry]))?,
            Arc::new(Explaining { revision: Arc::new(AtomicUsize::new(1)) }),
        )?)?;
        invoke(&db, "rebuild", "test")?;
        assert_eq!(invoke(&db, "explain", "test")?["explanations"][0]["stale"], false);
        *body.lock().expect("body") = "<urn:external> <urn:p> 2 .".into();
        assert_stale(&db, "test")
    })();
    stopped.store(true, Ordering::SeqCst);
    server.join().expect("server");
    result
}

#[test]
fn explanations_identify_generation_and_current_data_reconstruction() -> Result<()> {
    let mut db = Database::memory()?;
    db.register(Inference::with_reasoner(
        config("http://127.0.0.1:1", json!([job("test", &["urn:data"])]))?,
        Arc::new(Explaining { revision: Arc::new(AtomicUsize::new(1)) }),
    )?)?;
    let first = invoke(&db, "rebuild", "test")?;
    let explanation = invoke(&db, "explain", "test")?;
    let proof = &explanation["explanations"][0];
    assert_eq!(proof["generation"], first["generation"]);
    assert_eq!(proof["explanation_mode"], "reconstructed-current-data");
    assert_eq!(proof["recorded_proof"], false);
    assert_eq!(proof["published_source_token"], proof["evaluated_source_token"]);
    db.update("INSERT DATA { GRAPH <urn:data> { <urn:new> <urn:p> 1 } }")?;
    let changed = invoke(&db, "explain", "test")?;
    assert_ne!(
        changed["explanations"][0]["published_source_token"],
        changed["explanations"][0]["evaluated_source_token"]
    );
    assert_ne!(invoke(&db, "rebuild", "test")?["generation"], first["generation"]);
    Ok(())
}
