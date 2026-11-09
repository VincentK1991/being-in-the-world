//! HTTP client for the gateway. It only ever calls read endpoints: dataset listing, `/query`,
//! `/api/status`, `/api/plugins`, and the plugin operations in [`READ_OPERATIONS`].
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::time::Duration;

/// Plugin operations this server may call. Everything else (rebuild, validate, test, updates) is
/// refused here, whatever a tool asks for.
pub const READ_OPERATIONS: &[(&str, &str)] = &[
    ("dataset-summary", "count"),
    ("semantic-search", "status"),
    ("inference", "jobs"),
    ("inference", "status"),
    ("inference", "lineage"),
    ("inference", "explain"),
    ("shacl", "jobs"),
    ("shacl", "status"),
    ("federation", "status"),
    ("federation", "query"),
    ("virtual-graph", "status"),
];

/// The most bytes read from any one response; larger answers are cut off and flagged.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct Gateway {
    base: String,
    http: reqwest::Client,
}

pub struct Body {
    pub text: String,
    pub truncated: bool,
}

/// Dataset names go into URL paths, so only plain names are accepted.
pub fn check_dataset(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "invalid dataset name {name:?}; use a name from list_datasets"
    );
    Ok(())
}

/// The store has no authentication, so this server only talks to a gateway on this machine.
pub fn check_loopback(base: &str) -> Result<()> {
    let url = reqwest::Url::parse(base).with_context(|| format!("invalid gateway URL {base}"))?;
    ensure!(url.scheme() == "http" || url.scheme() == "https", "gateway URL must be http(s)");
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    ensure!(
        loopback,
        "gateway URL must be a loopback address (127.0.0.1, ::1 or localhost), got {host}"
    );
    Ok(())
}

impl Gateway {
    pub fn new(base: &str, timeout: Duration) -> Result<Self> {
        check_loopback(base)?;
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { base: base.trim_end_matches('/').to_owned(), http })
    }

    async fn read(response: reqwest::Response) -> Result<(reqwest::StatusCode, Body)> {
        let status = response.status();
        let mut response = response;
        let mut bytes = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                bytes.extend_from_slice(&chunk[..MAX_RESPONSE_BYTES - bytes.len()]);
                truncated = true;
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, Body { text: String::from_utf8_lossy(&bytes).into_owned(), truncated }))
    }

    async fn json(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let (status, body) = Self::read(request.send().await?).await?;
        if !status.is_success() {
            bail!("{}", body.text.trim());
        }
        ensure!(!body.truncated, "response too large");
        Ok(serde_json::from_str(&body.text)?)
    }

    pub async fn datasets(&self) -> Result<Value> {
        self.json(self.http.get(format!("{}/api/datasets", self.base))).await
    }

    fn dataset_url(&self, dataset: &str, rest: &str) -> Result<String> {
        check_dataset(dataset)?;
        Ok(format!("{}/d/{dataset}{rest}", self.base))
    }

    pub async fn status(&self, dataset: &str) -> Result<Value> {
        self.json(self.http.get(self.dataset_url(dataset, "/api/status")?)).await
    }

    pub async fn plugins(&self, dataset: &str) -> Result<Value> {
        self.json(self.http.get(self.dataset_url(dataset, "/api/plugins")?)).await
    }

    pub async fn operation(
        &self,
        dataset: &str,
        plugin: &str,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        ensure!(
            READ_OPERATIONS.contains(&(plugin, operation)),
            "{plugin}/{operation} is not a read-only operation"
        );
        let url =
            self.dataset_url(dataset, &format!("/api/plugins/{plugin}/operations/{operation}"))?;
        self.json(self.http.post(url).json(&input)).await
    }

    /// Run a query on the dataset's own endpoint. `accept` picks the result format.
    pub async fn query(&self, dataset: &str, query: &str, accept: &str) -> Result<Body> {
        let request = self
            .http
            .post(self.dataset_url(dataset, "/query")?)
            .header("accept", accept)
            .form(&[("query", query)]);
        let (status, body) = Self::read(request.send().await?).await?;
        if !status.is_success() {
            bail!("{}", body.text.trim());
        }
        Ok(body)
    }

    /// A SELECT or ASK whose JSON result must arrive whole.
    pub async fn select(&self, dataset: &str, query: &str) -> Result<Value> {
        let body = self.query(dataset, query, "application/sparql-results+json").await?;
        ensure!(!body.truncated, "result too large; narrow the query");
        Ok(serde_json::from_str(&body.text)?)
    }
}
