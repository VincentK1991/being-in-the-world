use super::*;
use std::io::{BufRead, Read, Write};

fn server(status: u16, response: Value) -> (String, std::thread::JoinHandle<Value>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        let mut reader = std::io::BufReader::new(&mut stream);
        let mut first = String::new();
        reader.read_line(&mut first).expect("request");
        assert_eq!(first, "POST /v1/embeddings HTTP/1.1\r\n");
        let mut length = 0;
        let mut auth = false;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header");
            if line == "\r\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().expect("length");
            }
            if lower.starts_with("authorization: bearer ") {
                auth = true;
            }
        }
        assert!(auth);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("body");
        let body = serde_json::from_slice(&body).expect("json");
        let response = response.to_string();
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).expect("reply");
        body
    });
    (url, worker)
}
fn adapter(url: &str) -> Result<Arc<dyn EmbeddingProvider>> {
    // PATH is an existing, non-secret test credential. No environment mutation.
    OpenAiFactory.create(&json!({"base_url":url,"model":"test-model","dimensions":2,"api_key_env":"PATH","revision":"test-v1"}))
}
#[test]
fn openai_sends_contract_and_restores_input_order() -> Result<()> {
    let (url, worker) = server(
        200,
        json!({"model":"test-model","data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}),
    );
    let provider = adapter(&url)?;
    let profile = provider.health()?.profile;
    assert_eq!(
        provider.embed(&["first".into(), "second".into()], &profile)?,
        vec![vec![1.0, 0.0], vec![0.0, 1.0]]
    );
    let request = worker.join().expect("server");
    assert_eq!(
        request,
        json!({"model":"test-model","dimensions":2,"encoding_format":"float","input":["first","second"]})
    );
    Ok(())
}
#[test]
fn malformed_responses_and_http_errors_fail_without_echoing_text() -> Result<()> {
    let cases = [
        (200, json!({"model":"wrong","data":[]})),
        (
            200,
            json!({"model":"test-model","data":[{"index":0,"embedding":[1.0,0.0]},{"index":0,"embedding":[1.0,0.0]}]}),
        ),
        (
            200,
            json!({"model":"test-model","data":[{"index":0,"embedding":[1.0]},{"index":1,"embedding":[1.0]}]}),
        ),
        (
            200,
            json!({"model":"test-model","data":[{"index":0,"embedding":[0.0,0.0]},{"index":1,"embedding":[1.0,0.0]}]}),
        ),
        (429, json!({"error":"PRIVATE_INPUT_AND_KEY"})),
        (401, json!({"error":"PRIVATE_INPUT_AND_KEY"})),
    ];
    for (status, response) in cases {
        let (url, worker) = server(status, response);
        let provider = adapter(&url)?;
        let error = provider
            .embed(&["one".into(), "two".into()], &provider.health()?.profile)
            .expect_err("invalid");
        assert!(!format!("{error:#}").contains("PRIVATE_INPUT_AND_KEY"));
        worker.join().expect("server");
    }
    Ok(())
}
#[test]
fn identity_and_input_policy() -> Result<()> {
    let provider = adapter("http://127.0.0.1:1/v1")?;
    let profile = provider.health()?.profile;
    assert!(provider.embed(&["x".repeat(8001)], &profile).is_err());
    let mut changed = profile.clone();
    changed.dimension = 3;
    assert!(provider.embed(&["text".into()], &changed).is_err());
    for url in [
        "http://example.com/v1",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=secret",
    ] {
        assert!(adapter(url).is_err());
    }
    let absent = OpenAiFactory.create(&json!({"model":"test","dimensions":2,"revision":"1","api_key_env":"RDF_TEST_MISSING_CREDENTIAL_8934723"}))?;
    assert!(
        absent
            .embed(&["text".into()], &absent.health()?.profile)
            .expect_err("missing key")
            .to_string()
            .contains("credential")
    );
    Ok(())
}
