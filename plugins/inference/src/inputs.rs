//! External inputs: RDF a job reads from a SPARQL sidecar in addition to its local source graphs.
//!
//! An input is a CONSTRUCT run against a loopback endpoint, so a rule or reasoner can work over data that is
//! not stored in the dataset: a virtual graph (Ontop over Postgres) or the federation sidecar (SERVICE across
//! datasets, other plugins' outputs included). The fetched triples are merged into the reasoner's input and
//! are never written to the store; only the deductions are published. Their digest is recorded so `status`
//! can tell when the outside data has changed.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spargebra::{Query, SparqlParser};
use std::{io::Read, time::Duration};

const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// A SPARQL Protocol endpoint such as Ontop: `POST` the query, `Accept: text/turtle` (converted to N-Triples).
    Sparql,
    /// The federation sidecar: `POST {"query", "source"?, "accept"}`; SERVICE clauses must be on its allowlist.
    Federation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalInput {
    pub name: String,
    pub kind: InputKind,
    /// Loopback HTTP endpoint, e.g. http://127.0.0.1:18090/sparql
    pub endpoint: String,
    /// A CONSTRUCT or DESCRIBE query.
    pub query: String,
    /// Federation only: endpoint that patterns outside SERVICE run against.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
}
fn timeout() -> u64 {
    60
}
/// What was fetched, recorded in the derived graph's metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    pub name: String,
    pub kind: InputKind,
    pub query_digest: String,
    pub triples: usize,
    pub digest: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub fetched_at: u64,
    #[serde(skip)]
    pub ntriples: String,
}
impl Fetched {
    pub fn record(&self) -> Value {
        json!({"name": self.name, "kind": self.kind, "query_digest": self.query_digest, "triples": self.triples, "digest": self.digest,"etag":self.etag,"last_modified":self.last_modified,"fetched_at":self.fetched_at,"coherence":"best-effort-observation"})
    }
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

impl ExternalInput {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 64
                && self.name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "invalid input name"
        );
        let url = reqwest::Url::parse(&self.endpoint)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str().is_some_and(|host| host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "input endpoint must be a loopback HTTP address"
        );
        ensure!((1..=600).contains(&self.timeout_secs), "input timeout_secs must be 1 to 600");
        ensure!(
            self.source.is_none() || self.kind == InputKind::Federation,
            "source applies only to federation inputs"
        );
        ensure!(self.query.len() <= 64 * 1024, "input query exceeds 64 KiB");
        let query = SparqlParser::new().parse_query(&self.query).context("invalid input query")?;
        ensure!(
            matches!(query, Query::Construct { .. } | Query::Describe { .. }),
            "an input must be a CONSTRUCT or DESCRIBE query"
        );
        Ok(())
    }

    /// Runs the query and returns its triples as sorted, de-duplicated N-Triples with blank nodes scoped to this input.
    pub fn fetch(&self, index: usize) -> Result<Fetched> {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(self.timeout_secs))
            .build()?;
        let request = match self.kind {
            InputKind::Sparql => client
                .post(&self.endpoint)
                .header("Content-Type", "application/sparql-query")
                .header("Accept", "text/turtle")
                .body(self.query.clone()),
            InputKind::Federation => {
                let mut body = json!({"query": self.query, "accept": "application/n-triples"});
                if let Some(source) = &self.source {
                    body["source"] = json!(source);
                }
                client.post(&self.endpoint).json(&body)
            }
        };
        let response =
            request.send().with_context(|| format!("input {} is unavailable", self.name))?;
        let status = response.status();
        let etag = response.headers().get("etag").and_then(|h| h.to_str().ok()).map(str::to_owned);
        let last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        response.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BYTES, "input {} exceeds 16 MiB", self.name);
        let text = String::from_utf8(bytes.to_vec()).context("input returned invalid UTF-8")?;
        ensure!(
            status.is_success(),
            "input {} returned {status}: {}",
            self.name,
            text.chars().take(500).collect::<String>()
        );
        let text = if self.kind == InputKind::Sparql { turtle_to_ntriples(&text)? } else { text };
        let text = canonical_remote(&text)?;
        let mut lines: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| scope_blank_nodes(line, &format!("i{index}")))
            .collect();
        lines.sort();
        lines.dedup();
        let ntriples = lines.join("\n");
        Ok(Fetched {
            name: self.name.clone(),
            etag,
            last_modified,
            fetched_at: super::now(),
            kind: self.kind,
            query_digest: digest(&self.query),
            triples: lines.len(),
            digest: digest(&ntriples),
            ntriples,
        })
    }
}

/// Endpoints such as Ontop answer CONSTRUCT in Turtle; the reasoners take N-Triples.
fn turtle_to_ntriples(turtle: &str) -> Result<String> {
    let mut out = String::new();
    for triple in oxttl::TurtleParser::new().for_slice(turtle.as_bytes()) {
        out.push_str(&format!("{} .\n", triple.context("input returned invalid Turtle")?));
    }
    Ok(out)
}

/// Renames subject and object blank nodes of one N-Triples line so inputs and the local snapshot never share labels.
pub fn scope_blank_nodes(line: &str, scope: &str) -> String {
    let rename = |token: &str| format!("_:{scope}_{}", &token[2..]);
    let mut line = line.to_owned();
    if line.starts_with("_:") {
        let end = line.find(char::is_whitespace).unwrap_or(line.len());
        line = format!("{}{}", rename(&line[..end]), &line[end..]);
    }
    let body = line.trim_end().trim_end_matches('.').trim_end();
    if let Some(start) = body.rfind(char::is_whitespace) {
        let token = &body[start + 1..];
        // A literal ends with a quote, language tag or datatype IRI and so never looks like `_:label`.
        if token.starts_with("_:") && !token.contains(['"', '<']) {
            line = format!("{} {} .", &body[..start], rename(token));
        }
    }
    line
}

/// Merges local snapshot triples with fetched inputs for the reasoner.
pub fn merge(local: &str, inputs: &[Fetched]) -> Result<String> {
    let mut merged = String::from(local);
    for input in inputs {
        if !merged.is_empty() && !input.ntriples.is_empty() {
            merged.push('\n');
        }
        merged.push_str(&input.ntriples);
    }
    ensure!(merged.len() <= 2 * MAX_BYTES, "reasoner input exceeds 32 MiB");
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn blank_nodes_are_scoped_but_literals_are_untouched() {
        assert_eq!(scope_blank_nodes("_:a <urn:p> _:b .", "i0"), "_:i0_a <urn:p> _:i0_b .");
        assert_eq!(
            scope_blank_nodes("<urn:s> <urn:p> \"keep _:b\" .", "i0"),
            "<urn:s> <urn:p> \"keep _:b\" ."
        );
        assert_eq!(scope_blank_nodes("_:a <urn:p> \"x\"@en .", "i1"), "_:i1_a <urn:p> \"x\"@en .");
        assert_eq!(
            scope_blank_nodes("<urn:s> <urn:p> <urn:o> .", "i0"),
            "<urn:s> <urn:p> <urn:o> ."
        );
    }
    fn input(kind: InputKind, endpoint: &str, query: &str) -> ExternalInput {
        ExternalInput {
            name: "x".into(),
            kind,
            endpoint: endpoint.into(),
            query: query.into(),
            source: None,
            timeout_secs: 5,
        }
    }
    #[test]
    fn inputs_must_be_loopback_construct_queries() {
        let construct = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
        assert!(
            input(InputKind::Sparql, "http://127.0.0.1:1/sparql", construct).validate().is_ok()
        );
        for bad in [
            "http://example.org/sparql",
            "https://127.0.0.1:1/sparql",
            "http://u@127.0.0.1/x",
            "http://127.0.0.1/x?y=1",
        ] {
            assert!(input(InputKind::Sparql, bad, construct).validate().is_err(), "{bad}");
        }
        assert!(
            input(InputKind::Sparql, "http://127.0.0.1:1/s", "SELECT * WHERE { ?s ?p ?o }")
                .validate()
                .is_err()
        );
        assert!(
            input(InputKind::Sparql, "http://127.0.0.1:1/s", "INSERT DATA { <a:a> <a:b> <a:c> }")
                .validate()
                .is_err()
        );
        let mut with_source = input(InputKind::Sparql, "http://127.0.0.1:1/s", construct);
        with_source.source = Some("http://x/q".into());
        assert!(with_source.validate().is_err());
    }
    /// One-shot HTTP server returning `body`; reports the request it received.
    fn serve(
        body: &'static str,
        status: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).expect("read");
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/n-triples\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).expect("write");
            request
        });
        (format!("http://{address}/sparql"), handle)
    }
    #[test]
    fn fetch_sorts_dedups_scopes_blank_nodes_and_digests() -> Result<()> {
        let (url, server) =
            serve("@prefix u: <urn:> .\nu:b u:p _:n .\nu:a u:p u:o , u:o .\n", "200 OK");
        let fetched =
            input(InputKind::Sparql, &url, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").fetch(3)?;
        let request = server.join().expect("server");
        assert!(request.to_ascii_lowercase().contains("accept: text/turtle"), "{request}");
        assert!(request.contains("CONSTRUCT"));
        assert_eq!(fetched.triples, 2);
        assert_eq!(fetched.ntriples.lines().count(), 2);
        assert!(fetched.ntriples.contains("<urn:a> <urn:p> <urn:o> ."), "{}", fetched.ntriples);
        assert!(fetched.ntriples.contains("_:i3_"), "blank nodes are scoped: {}", fetched.ntriples);
        assert_eq!(fetched.digest, digest(&fetched.ntriples));
        assert_eq!(merge("<urn:l> <urn:p> <urn:o> .", &[fetched])?.lines().count(), 3);
        Ok(())
    }
    #[test]
    fn failing_inputs_report_the_cause() {
        let (url, server) = serve("boom", "500 Internal Server Error");
        let error = input(InputKind::Sparql, &url, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
            .fetch(0)
            .expect_err("500");
        server.join().expect("server");
        assert!(error.to_string().contains("500") && error.to_string().contains("boom"), "{error}");
        assert!(
            input(
                InputKind::Sparql,
                "http://127.0.0.1:1/sparql",
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"
            )
            .fetch(0)
            .is_err()
        );
    }
}

/// Bound the costly symmetric-blank-node case explicitly; never compare changing
/// remote blank-node labels as if they were persistent identifiers.
fn canonical_remote(text: &str) -> Result<String> {
    use oxrdf::dataset::{CanonicalizationAlgorithm, CanonicalizationHashAlgorithm};
    use oxrdf::{Dataset, GraphName, NamedOrBlankNode, Quad, Term};
    let mut graph = Dataset::new();
    let mut blanks = std::collections::BTreeSet::new();
    for (count, t) in oxttl::NTriplesParser::new().for_slice(text.as_bytes()).enumerate() {
        ensure!(count < 100_000, "remote input exceeds 100000 triples");
        let t = t?;
        if let NamedOrBlankNode::BlankNode(b) = &t.subject {
            blanks.insert(b.as_str().to_owned());
        }
        if let Term::BlankNode(b) = &t.object {
            blanks.insert(b.as_str().to_owned());
        }
        ensure!(
            blanks.len() <= 8,
            "remote canonicalization exceeds the 8 blank-node safety limit; use stable IRIs"
        );
        graph.insert(Quad::new(t.subject, t.predicate, t.object, GraphName::DefaultGraph).as_ref());
    }
    graph.canonicalize(CanonicalizationAlgorithm::Rdfc10 {
        hash_algorithm: CanonicalizationHashAlgorithm::Sha256,
    });
    let mut lines = graph
        .iter()
        .map(|q| format!("{} {} {} .", q.subject, q.predicate, q.object))
        .collect::<Vec<_>>();
    lines.sort();
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod canonical_tests {
    use super::*;
    #[test]
    fn remote_blank_node_labels_do_not_affect_freshness() -> Result<()> {
        let a = canonical_remote("_:a <urn:p> _:b .\n_:b <urn:q> \"hello\"@en .")?;
        let b = canonical_remote("_:second <urn:q> \"hello\"@en .\n_:first <urn:p> _:second .")?;
        assert_eq!(a, b);
        assert_ne!(a, canonical_remote("_:a <urn:p> _:b .\n_:b <urn:q> \"other\"@en .")?);
        let excessive =
            (0..9).map(|i| format!("_:n{i} <urn:p> 1 .")).collect::<Vec<_>>().join("\n");
        assert!(canonical_remote(&excessive).is_err());
        Ok(())
    }
    #[test]
    fn oversized_remote_body_is_stopped_before_parsing() -> Result<()> {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let endpoint = format!("http://{}/sparql", listener.local_addr()?);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0; 8192];
            let _ = stream.read(&mut request);
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/turtle\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",MAX_BYTES+8192).expect("header");
            let chunk = [b'a'; 8192];
            for _ in 0..(MAX_BYTES / 8192 + 1) {
                if stream.write_all(&chunk).is_err() {
                    break;
                }
            }
        });
        let input: ExternalInput = serde_json::from_value(
            json!({"name":"large","kind":"sparql","endpoint":endpoint,"query":"CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"}),
        )?;
        let result = input.fetch(0);
        server.join().expect("server");
        assert!(format!("{:#}", result.expect_err("bounded body")).contains("exceeds 16 MiB"));
        Ok(())
    }
}
