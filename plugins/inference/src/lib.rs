//! Inference orchestration through generic snapshot/publication capabilities.
mod inputs;
use anyhow::{Context, Result, bail, ensure};
pub use inputs::{ExternalInput, Fetched, InputKind};
use rdf_plugin_api::{Manifest, Plugin, ReadContext, SDK_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, ProviderConfig>,
    pub jobs: Vec<Job>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub name: String,
    pub profile: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    pub source_graphs: Vec<String>,
    /// Extra RDF fetched from a sidecar at rebuild time (virtual graph or federation); never stored, only deduced from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ExternalInput>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub kind: String,
    pub config: Value,
}
pub trait ReasonerFactory: Send + Sync {
    fn create(&self, config: &Value) -> Result<Arc<dyn Reasoner>>;
}
#[derive(Default)]
pub struct ReasonerRegistry {
    factories: std::collections::BTreeMap<String, Arc<dyn ReasonerFactory>>,
}
impl ReasonerRegistry {
    pub fn register(&mut self, kind: &str, factory: impl ReasonerFactory + 'static) -> Result<()> {
        ensure!(
            !kind.is_empty() && !self.factories.contains_key(kind),
            "duplicate or empty reasoner kind"
        );
        self.factories.insert(kind.into(), Arc::new(factory));
        Ok(())
    }
    fn create(&self, config: &ProviderConfig) -> Result<Arc<dyn Reasoner>> {
        self.factories
            .get(&config.kind)
            .context("unregistered reasoner kind")?
            .create(&config.config)
    }
}
/// Providers compute deductions only; the host owns storage and publication.
pub trait Reasoner: Send + Sync {
    fn identity(&self) -> Result<Value>;
    fn infer(&self, profile: &str, ntriples: &str) -> Result<(String, Value)>;
    fn infer_rules(
        &self,
        profile: &str,
        ntriples: &str,
        rules: &[String],
    ) -> Result<(String, Value)> {
        ensure!(rules.is_empty(), "provider does not accept custom rules");
        self.infer(profile, ntriples)
    }
}
pub struct Inference {
    config: Config,
    reasoners: std::collections::BTreeMap<String, Arc<dyn Reasoner>>,
    gate: Mutex<()>,
}
struct HttpReasoner {
    endpoint: String,
    client: reqwest::blocking::Client,
}
impl HttpReasoner {
    fn response(&self, response: reqwest::blocking::Response) -> Result<Value> {
        let status = response.status();
        let value: Value = response.json().context("invalid reasoner response")?;
        ensure!(
            status.is_success(),
            "reasoner returned {status}: {}",
            value["error"].as_str().unwrap_or("request failed")
        );
        Ok(value)
    }
}
impl Reasoner for HttpReasoner {
    fn identity(&self) -> Result<Value> {
        self.response(self.client.get(format!("{}/health", self.endpoint)).send()?)
    }
    fn infer(&self, profile: &str, ntriples: &str) -> Result<(String, Value)> {
        self.infer_rules(profile, ntriples, &[])
    }
    fn infer_rules(
        &self,
        profile: &str,
        ntriples: &str,
        rules: &[String],
    ) -> Result<(String, Value)> {
        let mut body = json!({"profile":profile,"ntriples":ntriples});
        if !rules.is_empty() {
            body["rules"] = json!(rules);
        }
        let result = self
            .response(self.client.post(format!("{}/infer", self.endpoint)).json(&body).send()?)?;
        Ok((
            result["ntriples"].as_str().context("missing derived triples")?.into(),
            result["reasoner"].clone(),
        ))
    }
}
pub struct HttpReasonerFactory;
impl ReasonerFactory for HttpReasonerFactory {
    fn create(&self, config: &Value) -> Result<Arc<dyn Reasoner>> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Endpoint {
            endpoint: String,
        }
        let config: Endpoint = serde_json::from_value(config.clone())?;
        let url = reqwest::Url::parse(&config.endpoint)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str().is_some_and(|s| s
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()))
                && url.path() == "/"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "reasoner endpoint must be a loopback HTTP origin"
        );
        Ok(Arc::new(HttpReasoner {
            endpoint: config.endpoint.trim_end_matches('/').into(),
            client: reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(70))
                .build()?,
        }))
    }
}
impl Inference {
    /// Compatibility constructor for the original OWL-RL sidecar configuration.
    pub fn new(config: Config) -> Result<Self> {
        let mut registry = ReasonerRegistry::default();
        registry.register("owlrl", HttpReasonerFactory)?;
        Self::with_registry(config, &registry)
    }
    pub fn with_registry(mut config: Config, registry: &ReasonerRegistry) -> Result<Self> {
        ensure!(
            config.endpoint.is_none() || config.providers.is_empty(),
            "legacy endpoint cannot be combined with providers"
        );
        if let Some(endpoint) = &config.endpoint {
            config.providers.insert(
                "legacy".into(),
                ProviderConfig { kind: "owlrl".into(), config: json!({"endpoint":endpoint}) },
            );
            for job in &mut config.jobs {
                ensure!(
                    job.provider.is_none(),
                    "legacy endpoint cannot be combined with job provider"
                );
                job.provider = Some("legacy".into());
            }
        }
        let mut reasoners = std::collections::BTreeMap::new();
        for (name, provider) in &config.providers {
            ensure!(!name.is_empty(), "empty provider name");
            reasoners.insert(name.clone(), registry.create(provider)?);
        }
        Self::validated(config, reasoners)
    }
    pub fn with_reasoner(mut config: Config, reasoner: Arc<dyn Reasoner>) -> Result<Self> {
        ensure!(
            config.providers.is_empty() && config.jobs.iter().all(|j| j.provider.is_none()),
            "with_reasoner expects legacy jobs"
        );
        for job in &mut config.jobs {
            job.provider = Some("injected".into());
        }
        config.providers.insert(
            "injected".into(),
            ProviderConfig { kind: "injected".into(), config: json!({"endpoint":config.endpoint}) },
        );
        Self::validated(config, std::collections::BTreeMap::from([("injected".into(), reasoner)]))
    }
    fn validated(
        config: Config,
        reasoners: std::collections::BTreeMap<String, Arc<dyn Reasoner>>,
    ) -> Result<Self> {
        ensure!(!config.jobs.is_empty(), "configure at least one inference job");
        let mut names = std::collections::HashSet::new();
        for job in &config.jobs {
            ensure!(
                job.provider.as_ref().is_some_and(|name| reasoners.contains_key(name)),
                "job must reference a configured provider"
            );
            ensure!(
                job.rules.len() <= 64
                    && job.rules.iter().map(String::len).sum::<usize>() <= 1024 * 1024,
                "rules exceed 64 documents or 1 MiB"
            );
            ensure!(
                !job.name.is_empty()
                    && job.name.len() <= 64
                    && job
                        .name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                    && names.insert(&job.name),
                "invalid or duplicate inference job"
            );
            ensure!(!job.profile.is_empty(), "unsupported inference profile");
            ensure!(job.inputs.len() <= 8, "a job accepts at most 8 inputs");
            let mut input_names = std::collections::HashSet::new();
            for input in &job.inputs {
                input.validate()?;
                ensure!(input_names.insert(&input.name), "duplicate input name");
            }
            ensure!(
                !job.source_graphs.is_empty() && job.source_graphs.len() <= 64,
                "select 1 to 64 source graphs"
            );
            for graph in &job.source_graphs {
                ensure!(
                    !graph.starts_with("urn:rdftool:derived:"),
                    "derived graphs cannot be source graphs"
                );
                if graph != "default" {
                    oxiri::Iri::parse(graph.as_str())?;
                }
            }
        }
        Ok(Self { config, reasoners, gate: Mutex::new(()) })
    }
}
fn fetch_inputs(job: &Job) -> Result<Vec<Fetched>> {
    job.inputs.iter().enumerate().map(|(index, input)| input.fetch(index)).collect()
}
impl Plugin for Inference {
    fn facets(&self) -> Vec<rdf_plugin_api::Facet> {
        self.config
            .jobs
            .iter()
            .map(|job| rdf_plugin_api::Facet::new("job", job.name.clone()))
            .collect()
    }
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "inference".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["rebuild".into(), "status".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        let object = input.as_object().context("expected object")?;
        ensure!(object.len() == 1 && object.contains_key("job"), "expected only job argument");
        let name = input["job"].as_str().context("job required")?;
        let job = self
            .config
            .jobs
            .iter()
            .find(|job| job.name == name)
            .context("unknown inference job")?;
        let provider = job.provider.as_ref().expect("validated provider");
        let reasoner = &self.reasoners[provider];
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(job, &self.config.providers[provider]))?)
        );
        match operation {
            "rebuild" => {
                let _guard = self
                    .gate
                    .try_lock()
                    .map_err(|_| anyhow::anyhow!("inference rebuild already running"))?;
                let source = context.snapshot(&job.source_graphs)?;
                let identity = reasoner.identity()?;
                ensure!(
                    identity["profiles"].as_array().is_some_and(|profiles| profiles
                        .iter()
                        .any(|profile| profile == &job.profile)),
                    "reasoner does not support requested profile"
                );
                let fetched = fetch_inputs(job)?;
                let merged = inputs::merge(&source.ntriples, &fetched)?;
                let (triples, actual) = reasoner.infer_rules(&job.profile, &merged, &job.rules)?;
                ensure!(actual == identity, "reasoner identity changed during rebuild");
                let mut metadata =
                    json!({"config_digest":fingerprint,"reasoner":identity,"profile":job.profile});
                if !fetched.is_empty() {
                    metadata["inputs"] = fetched.iter().map(Fetched::record).collect();
                }
                let mut result = context.publish(name, &source, &triples, metadata)?;
                result["state"] = json!("ready");
                Ok(result)
            }
            "status" => {
                let mut status = context.derived_status(name)?;
                if status["state"] != "missing"
                    && (status["metadata"]["config_digest"] != fingerprint
                        || status["metadata"]["reasoner"] != reasoner.identity()?)
                {
                    status["state"] = json!("stale");
                }
                // Outside data is not part of the store's source token, so re-read it and compare digests.
                if status["state"] == "ready" && !job.inputs.is_empty() {
                    match fetch_inputs(job) {
                        Ok(now) => {
                            let recorded = &status["metadata"]["inputs"];
                            let changed: Vec<_> = now
                                .iter()
                                .filter(|input| {
                                    !recorded.as_array().is_some_and(|all| {
                                        all.iter().any(|r| {
                                            r["name"] == input.name && r["digest"] == input.digest
                                        })
                                    })
                                })
                                .map(|input| input.name.clone())
                                .collect();
                            if !changed.is_empty() {
                                status["state"] = json!("stale");
                                status["stale_inputs"] = json!(changed);
                            }
                        }
                        Err(error) => {
                            status["state"] = json!("stale");
                            status["input_error"] = json!(format!("{error:#}"));
                        }
                    }
                }
                Ok(status)
            }
            _ => bail!("unknown inference operation"),
        }
    }
}
