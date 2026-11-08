//! Virtual graphs and SPARQL federation as plugins over two sidecars.
//!
//! * `virtual-graph`: an Ontop endpoint that exposes a relational source (R2RML over Postgres) as RDF.
//!   Rust does no SQL work; the plugin reports status and runs read queries against the endpoint.
//! * `federation`: a Comunica endpoint that evaluates standard `SERVICE` clauses. The plugin refuses
//!   any `SERVICE` target that is not on the configured allowlist before the sidecar sees the query.
//!
//! Both talk only to loopback sidecars, never follow redirects, and never write remote data.
use anyhow::{Context, Result, bail, ensure};
use rdf_plugin_api::{Manifest, Plugin, ReadContext, SDK_VERSION};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spargebra::{
    Query, SparqlParser,
    algebra::{AggregateExpression, Expression as E, GraphPattern as G, OrderExpression},
    term::NamedNodePattern,
};
use std::{collections::BTreeSet, time::Duration};

#[cfg(test)]
mod tests;

const MAX_QUERY_BYTES: usize = 64 * 1024;

fn sidecar(url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        parsed.scheme() == "http"
            && parsed.host_str().is_some_and(|host| host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "sidecar URL must be a loopback HTTP address, e.g. http://127.0.0.1:18095"
    );
    Ok(url.trim_end_matches('/').into())
}
fn http(timeout_secs: u64) -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout_secs))
        // The sidecars (Node, Jetty) close idle connections after a few seconds; reqwest would
        // otherwise keep them for 90 s and sometimes reuse one the server just closed.
        .pool_idle_timeout(Duration::from_secs(2))
        .build()?)
}
/// Sends a read-only request, trying once more if the connection failed (not if it timed out):
/// a pooled connection can be closed by the sidecar at the moment it is reused.
fn send(
    request: reqwest::blocking::RequestBuilder,
    what: &str,
) -> Result<reqwest::blocking::Response> {
    let retry = request.try_clone();
    let outcome = match (request.send(), retry) {
        (Err(error), Some(again)) if !error.is_timeout() => again.send(),
        (first, _) => first,
    };
    outcome.with_context(|| format!("{what} unavailable"))
}
fn timeout_default() -> u64 {
    60
}
fn check_timeout(secs: u64) -> Result<()> {
    ensure!((1..=600).contains(&secs), "timeout_secs must be 1 to 600");
    Ok(())
}
fn parse(query: &str) -> Result<Query> {
    ensure!(
        !query.trim().is_empty() && query.len() <= MAX_QUERY_BYTES,
        "query must be nonempty and at most 64 KiB"
    );
    SparqlParser::new().parse_query(query).context("invalid SPARQL query")
}
fn only_query(input: &Value) -> Result<&str> {
    let object = input.as_object().context("operation expects a JSON object")?;
    ensure!(object.keys().all(|key| key == "query"), "unknown argument; expected only \"query\"");
    input["query"].as_str().context("query is required")
}

// ---- SERVICE targets ----
fn expression(e: &E, out: &mut Vec<NamedNodePattern>) {
    match e {
        E::Exists(g) => services(g, out),
        E::Or(a, b)
        | E::And(a, b)
        | E::Equal(a, b)
        | E::SameTerm(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Add(a, b)
        | E::Subtract(a, b)
        | E::Multiply(a, b)
        | E::Divide(a, b) => {
            expression(a, out);
            expression(b, out);
        }
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expression(a, out),
        E::In(a, args) => {
            expression(a, out);
            args.iter().for_each(|arg| expression(arg, out));
        }
        E::If(a, b, c) => {
            expression(a, out);
            expression(b, out);
            expression(c, out);
        }
        E::Coalesce(args) | E::FunctionCall(_, args) => {
            args.iter().for_each(|arg| expression(arg, out))
        }
        _ => {}
    }
}
/// Every SERVICE name in a graph pattern, including ones nested in sub-patterns and FILTER EXISTS.
fn services(g: &G, out: &mut Vec<NamedNodePattern>) {
    match g {
        G::Service { name, inner, .. } => {
            out.push(name.clone());
            services(inner, out);
        }
        G::Join { left, right }
        | G::Union { left, right }
        | G::Minus { left, right }
        | G::Lateral { left, right } => {
            services(left, out);
            services(right, out);
        }
        G::LeftJoin { left, right, expression: e } => {
            services(left, out);
            services(right, out);
            if let Some(e) = e {
                expression(e, out);
            }
        }
        G::Filter { inner, expr } => {
            services(inner, out);
            expression(expr, out);
        }
        G::Extend { inner, expression: e, .. } => {
            services(inner, out);
            expression(e, out);
        }
        G::Graph { inner, .. }
        | G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => services(inner, out),
        G::OrderBy { inner, expression: order } => {
            services(inner, out);
            for item in order {
                let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = item;
                expression(e, out);
            }
        }
        G::Group { inner, aggregates, .. } => {
            services(inner, out);
            for (_, aggregate) in aggregates {
                if let AggregateExpression::FunctionCall { expr, .. } = aggregate {
                    expression(expr, out);
                }
            }
        }
        _ => {}
    }
}
/// SERVICE endpoint IRIs a read query would contact; a variable endpoint is refused.
pub fn service_targets(query: &str) -> Result<Vec<String>> {
    let query = parse(query)?;
    let (Query::Select { pattern, .. }
    | Query::Construct { pattern, .. }
    | Query::Describe { pattern, .. }
    | Query::Ask { pattern, .. }) = &query;
    let mut found = Vec::new();
    services(pattern, &mut found);
    let mut targets = BTreeSet::new();
    for name in found {
        match name {
            NamedNodePattern::NamedNode(node) => {
                targets.insert(node.into_string());
            }
            NamedNodePattern::Variable(_) => {
                bail!("SERVICE needs a registered endpoint IRI, not a variable")
            }
        }
    }
    Ok(targets.into_iter().collect())
}

fn read_json(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let text = response.text()?;
    ensure!(
        status.is_success(),
        "sidecar returned {status}: {}",
        text.chars().take(600).collect::<String>()
    );
    // SPARQL JSON for SELECT/ASK; Turtle text for CONSTRUCT/DESCRIBE.
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({"turtle": text})))
}

// ---- virtual-graph (Ontop) ----
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualGraphConfig {
    /// Human-readable name of the virtual graph, e.g. "acme-postgres".
    pub name: String,
    /// Ontop SPARQL endpoint, e.g. http://127.0.0.1:18090/sparql (loopback only).
    pub endpoint: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "timeout_default")]
    pub timeout_secs: u64,
}
pub struct VirtualGraph {
    config: VirtualGraphConfig,
    client: Client,
}
impl VirtualGraph {
    pub fn new(mut config: VirtualGraphConfig) -> Result<Self> {
        ensure!(
            !config.name.is_empty() && config.name.len() <= 64,
            "virtual graph name must be 1 to 64 characters"
        );
        config.endpoint = sidecar(&config.endpoint)?;
        check_timeout(config.timeout_secs)?;
        let client = http(config.timeout_secs)?;
        Ok(Self { config, client })
    }
    fn run(&self, query: &str) -> Result<Value> {
        let accept = if matches!(parse(query)?, Query::Construct { .. } | Query::Describe { .. }) {
            "text/turtle"
        } else {
            "application/sparql-results+json"
        };
        let response = send(
            self.client
                .post(&self.config.endpoint)
                .header("Content-Type", "application/sparql-query")
                .header("Accept", accept)
                .body(query.to_owned()),
            "virtual graph sidecar",
        )?;
        read_json(response)
    }
}
impl Plugin for VirtualGraph {
    fn facets(&self) -> Vec<rdf_plugin_api::Facet> {
        vec![rdf_plugin_api::Facet::new("virtual-graph", self.config.name.clone())]
    }
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "virtual-graph".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["status".into(), "query".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, _context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        match operation {
            "status" => {
                ensure!(
                    input.as_object().is_some_and(|o| o.is_empty()),
                    "status expects an empty JSON object"
                );
                let started = std::time::Instant::now();
                let count = self.run("SELECT (COUNT(*) AS ?triples) WHERE { ?s ?p ?o }");
                Ok(match count {
                    Ok(result) => json!({
                        "name": self.config.name, "description": self.config.description, "endpoint": self.config.endpoint,
                        "state": "ready", "triples": result["results"]["bindings"][0]["triples"]["value"],
                        "latency_ms": started.elapsed().as_millis() as u64,
                    }),
                    Err(error) => json!({
                        "name": self.config.name, "endpoint": self.config.endpoint, "state": "unavailable", "error": error.to_string(),
                    }),
                })
            }
            "query" => {
                let query = only_query(&input)?;
                // The sidecar owns SQL generation; make sure it can only be asked a read query that stays inside it.
                ensure!(
                    service_targets(query)?.is_empty(),
                    "SERVICE is not available on a virtual graph; use the federation plugin"
                );
                self.run(query)
            }
            _ => bail!("unknown virtual-graph operation"),
        }
    }
}

// ---- federation (Comunica) ----
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederationConfig {
    /// Comunica sidecar, e.g. http://127.0.0.1:18095/sparql (loopback only).
    pub endpoint: String,
    /// Endpoint that patterns outside SERVICE are evaluated against, normally this dataset's own /query URL.
    #[serde(default)]
    pub local_source: Option<String>,
    /// SERVICE endpoint IRIs a query may contact. Exact match; the sidecar enforces the same list.
    pub allow: Vec<String>,
    #[serde(default = "timeout_default")]
    pub timeout_secs: u64,
}
pub struct Federation {
    config: FederationConfig,
    client: Client,
}
impl Federation {
    pub fn new(mut config: FederationConfig) -> Result<Self> {
        config.endpoint = sidecar(&config.endpoint)?;
        check_timeout(config.timeout_secs)?;
        ensure!(!config.allow.is_empty(), "allow at least one SERVICE endpoint");
        for iri in config.allow.iter().chain(config.local_source.iter()) {
            oxiri::Iri::parse(iri.as_str())
                .with_context(|| format!("invalid endpoint IRI: {iri}"))?;
            ensure!(
                iri.starts_with("http://") || iri.starts_with("https://"),
                "endpoint must be http(s): {iri}"
            );
        }
        let client = http(config.timeout_secs)?;
        Ok(Self { config, client })
    }
    fn health_url(&self) -> String {
        self.config.endpoint.trim_end_matches("/sparql").to_owned() + "/health"
    }
    fn allowed_url(&self) -> String {
        self.config.endpoint.trim_end_matches("/sparql").to_owned() + "/allowed"
    }
    /// Endpoints this dataset allows that the running sidecar would refuse. The sidecar enforces its
    /// own list, so a mismatch makes those queries fail; `None` when the sidecar cannot be asked.
    fn missing_in_sidecar(&self) -> Option<Vec<String>> {
        let response = self.client.get(self.allowed_url()).send().ok()?.error_for_status().ok()?;
        let theirs: Vec<String> =
            serde_json::from_value(response.json::<Value>().ok()?["allow"].clone()).ok()?;
        Some(missing(&self.config.allow, self.config.local_source.as_deref(), &theirs))
    }
    /// Refuses a query that would contact an endpoint outside the allowlist; returns the endpoints it will use.
    fn authorize(&self, query: &str) -> Result<Vec<String>> {
        let targets = service_targets(query)?;
        for target in &targets {
            ensure!(
                self.config.allow.contains(target),
                "endpoint is not registered for federation: {target}"
            );
        }
        ensure!(
            !targets.is_empty() || self.config.local_source.is_some(),
            "query has no SERVICE clause; run it directly on the dataset"
        );
        Ok(targets)
    }
}
impl Plugin for Federation {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "federation".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["status".into(), "query".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, _context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        match operation {
            "status" => {
                ensure!(
                    input.as_object().is_some_and(|o| o.is_empty()),
                    "status expects an empty JSON object"
                );
                let health = self
                    .client
                    .get(self.health_url())
                    .send()
                    .ok()
                    .and_then(|r| r.error_for_status().ok())
                    .and_then(|r| r.json::<Value>().ok());
                let missing = if health.is_some() { self.missing_in_sidecar() } else { None };
                // The sidecar enforces its own allowlist; entries only this dataset lists would be refused there.
                let state = match (&health, &missing) {
                    (None, _) => "unavailable",
                    (Some(_), Some(m)) if !m.is_empty() => "misconfigured",
                    _ => "ready",
                };
                Ok(json!({
                    "state": state,
                    "endpoint": self.config.endpoint, "engine": health.as_ref().map(|h| h["engine"].clone()),
                    "local_source": self.config.local_source, "allow": self.config.allow,
                    "missing_in_sidecar": missing,
                }))
            }
            "query" => {
                let query = only_query(&input)?;
                let services = self.authorize(query)?;
                let mut body = json!({"query": query});
                if let Some(source) = &self.config.local_source {
                    body["source"] = json!(source);
                }
                let response = send(
                    self.client.post(&self.config.endpoint).json(&body),
                    "federation sidecar",
                )?;
                let mut result = read_json(response)?;
                if let Some(object) = result.as_object_mut() {
                    object.insert("services".into(), json!(services));
                }
                Ok(result)
            }
            _ => bail!("unknown federation operation"),
        }
    }
}

/// Allowed endpoints (and the local source) that the sidecar's list does not contain.
fn missing(allow: &[String], local_source: Option<&str>, sidecar: &[String]) -> Vec<String> {
    let mut wanted: Vec<&str> = allow.iter().map(String::as_str).chain(local_source).collect();
    wanted.sort_unstable();
    wanted.dedup();
    wanted.into_iter().filter(|e| !sidecar.iter().any(|s| s == e)).map(str::to_owned).collect()
}
