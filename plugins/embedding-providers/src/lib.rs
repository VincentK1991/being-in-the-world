//! Built-in adapters. Applications register these; semantic search knows only the SDK.
use anyhow::{Context, Result, ensure};
use rdf_embedding_api::{
    EmbeddingProvider, Health, Profile, ProviderFactory, ProviderRegistry, validate,
};
use reqwest::{Method, blocking::Client};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub fn registry() -> Result<ProviderRegistry> {
    let mut registry = ProviderRegistry::default();
    registry.register("local", LocalFactory)?;
    registry.register("openai", OpenAiFactory)?;
    Ok(registry)
}
fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(180))
        .build()?)
}
fn endpoint(value: &str, local: bool) -> Result<String> {
    let url = reqwest::Url::parse(value)?;
    let loopback = url
        .host_str()
        .is_some_and(|h| h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && (if local {
                loopback && url.scheme() == "http" && url.path() == "/"
            } else {
                url.scheme() == "https" || (loopback && url.scheme() == "http")
            }),
        "embedding endpoint must use HTTPS (loopback HTTP allowed), without credentials, query or fragment"
    );
    Ok(value.trim_end_matches('/').into())
}
fn request(
    client: &Client,
    method: Method,
    url: String,
    body: Option<Value>,
    key: Option<String>,
) -> Result<Value> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    // Never surface response bodies or request details: providers can echo text or credentials.
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("embedding provider unavailable or request timed out"))?;
    ensure!(
        response.status().is_success(),
        "embedding provider returned HTTP {}",
        response.status()
    );
    response.json().map_err(|_| anyhow::anyhow!("invalid embedding provider JSON"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalConfig {
    url: String,
}
pub struct LocalFactory;
impl ProviderFactory for LocalFactory {
    fn create(&self, config: &Value) -> Result<Arc<dyn EmbeddingProvider>> {
        let config: LocalConfig = serde_json::from_value(config.clone())?;
        Ok(Arc::new(Local { url: endpoint(&config.url, true)?, client: client()? }))
    }
}
struct Local {
    url: String,
    client: Client,
}
impl Local {
    fn profile(&self, value: &Value) -> Result<Profile> {
        let dimension = value["dimension"].as_u64().context("local dimension missing")? as usize;
        ensure!((1..=65536).contains(&dimension), "invalid local dimension");
        let model =
            value["model"].as_str().filter(|x| !x.is_empty()).context("local model missing")?;
        ensure!(
            value["revision"].as_str().is_some_and(|x| !x.is_empty()),
            "local revision missing"
        );
        Ok(Profile {
            provider: "local-v1".into(),
            model: model.into(),
            dimension,
            identity: json!({"endpoint":self.url,"profile":value}),
        })
    }
}
impl EmbeddingProvider for Local {
    fn health(&self) -> Result<Health> {
        let value = request(&self.client, Method::GET, format!("{}/health", self.url), None, None)?;
        Ok(Health {
            profile: self.profile(&value["profile"])?,
            device: value["device"].as_str().context("local device missing")?.into(),
        })
    }
    fn embed(&self, texts: &[String], expected: &Profile) -> Result<Vec<Vec<f32>>> {
        let value = request(
            &self.client,
            Method::POST,
            format!("{}/embed", self.url),
            Some(json!({"texts":texts})),
            None,
        )?;
        ensure!(
            &self.profile(&value["profile"])? == expected,
            "embedding profile changed; rebuild required"
        );
        let vectors: Vec<Vec<f32>> =
            serde_json::from_value(value["vectors"].clone()).context("invalid local vectors")?;
        validate(&vectors, texts.len(), expected)?;
        Ok(vectors)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAiConfig {
    #[serde(default = "openai_url")]
    base_url: String,
    model: String,
    dimensions: usize,
    #[serde(default = "key_env")]
    api_key_env: String,
    /// Operator-controlled version for hosted aliases that are not immutable.
    revision: String,
    /// Conservative UTF-8 byte bound, not a tokenizer or truncation policy.
    #[serde(default = "input_limit")]
    max_input_bytes: usize,
}
fn openai_url() -> String {
    "https://api.openai.com/v1".into()
}
fn key_env() -> String {
    "OPENAI_API_KEY".into()
}
fn input_limit() -> usize {
    8000
}
pub struct OpenAiFactory;
impl ProviderFactory for OpenAiFactory {
    fn create(&self, config: &Value) -> Result<Arc<dyn EmbeddingProvider>> {
        let mut config: OpenAiConfig = serde_json::from_value(config.clone())?;
        config.base_url = endpoint(&config.base_url, false)?;
        ensure!(
            !config.model.trim().is_empty() && !config.revision.trim().is_empty(),
            "model and revision are required"
        );
        ensure!((1..=65536).contains(&config.dimensions), "invalid dimensions");
        ensure!((1..=8000).contains(&config.max_input_bytes), "max_input_bytes must be 1 to 8000");
        ensure!(
            !config.api_key_env.is_empty()
                && config.api_key_env.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "invalid credential environment name"
        );
        Ok(Arc::new(OpenAi { config, client: client()? }))
    }
}
struct OpenAi {
    config: OpenAiConfig,
    client: Client,
}
impl OpenAi {
    fn profile(&self) -> Profile {
        Profile {
            provider: "openai-compatible-v1".into(),
            model: self.config.model.clone(),
            dimension: self.config.dimensions,
            identity: json!({"endpoint":self.config.base_url,"revision":self.config.revision,"max_input_bytes":self.config.max_input_bytes,"preprocessing":"identity-reject-long-v1"}),
        }
    }
}
impl EmbeddingProvider for OpenAi {
    fn health(&self) -> Result<Health> {
        // Metadata only: status must neither send private text nor incur embedding charges.
        Ok(Health { profile: self.profile(), device: "remote".into() })
    }
    fn embed(&self, texts: &[String], expected: &Profile) -> Result<Vec<Vec<f32>>> {
        ensure!(&self.profile() == expected, "embedding configuration changed; rebuild required");
        ensure!(
            !texts.is_empty() && texts.len() <= 32,
            "embedding batch must contain 1 to 32 inputs"
        );
        ensure!(
            texts.iter().all(|s| !s.trim().is_empty() && s.len() <= self.config.max_input_bytes),
            "embedding input empty or exceeds max_input_bytes; no truncation performed"
        );
        let key = std::env::var(&self.config.api_key_env)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .context("embedding API credential environment variable is missing or empty")?;
        let value = request(
            &self.client,
            Method::POST,
            format!("{}/embeddings", self.config.base_url),
            Some(
                json!({"model":self.config.model,"dimensions":self.config.dimensions,"encoding_format":"float","input":texts}),
            ),
            Some(key),
        )?;
        ensure!(
            value["model"].as_str() == Some(self.config.model.as_str()),
            "embedding response model mismatch"
        );
        let data = value["data"].as_array().context("embedding data missing")?;
        ensure!(data.len() == texts.len(), "embedding batch length mismatch");
        let mut ordered = vec![None; texts.len()];
        for item in data {
            let index = item["index"].as_u64().context("embedding index missing")? as usize;
            ensure!(
                index < ordered.len() && ordered[index].is_none(),
                "duplicate or invalid embedding index"
            );
            ordered[index] = Some(
                serde_json::from_value::<Vec<f32>>(item["embedding"].clone())
                    .context("invalid embedding values")?,
            );
        }
        let vectors: Vec<_> =
            ordered.into_iter().collect::<Option<_>>().context("missing embedding index")?;
        validate(&vectors, texts.len(), expected)?;
        Ok(vectors)
    }
}
#[cfg(test)]
mod tests;
