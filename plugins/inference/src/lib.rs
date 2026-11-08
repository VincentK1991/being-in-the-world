//! Inference orchestration through generic snapshot/publication capabilities.
mod explain;
mod inputs;
mod rule_tests;
use anyhow::{Context, Result, bail, ensure};
pub use inputs::{ExternalInput, Fetched, InputKind};
use rdf_plugin_api::{Manifest, Plugin, ReadContext, SDK_VERSION};
pub use rule_tests::RuleTest;
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
    /// Checked by the `test` operation. Not serialized, so editing a test never makes the job stale.
    #[serde(default, skip_serializing)]
    pub tests: Vec<RuleTest>,
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
    /// How `target` (one N-Triples line) follows from `ntriples` under `rules`: one entry per derivation,
    /// each with the rule, the variable bindings and the triples it relied on. `None` when this
    /// reasoner records no justifications.
    fn explain(
        &self,
        _profile: &str,
        _ntriples: &str,
        _rules: &[String],
        _target: &str,
    ) -> Result<Option<Vec<Value>>> {
        Ok(None)
    }
    /// Whether `explain` can name rules and premises, so a UI knows before asking.
    fn explains(&self) -> bool {
        false
    }
}
/// The reasoner found the input inconsistent. Not a failure: the job keeps its last good output
/// and reports what clashed. Providers return it as an error so callers that ignore it still stop.
#[derive(Debug, Clone)]
pub struct Inconsistent {
    /// One entry per clash: at least `message`; `terms` names the resources involved; a provider
    /// that records justifications also gives `rule` and `premises`.
    pub clashes: Vec<Value>,
}
impl std::fmt::Display for Inconsistent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let first = self.clashes.first().and_then(|c| c["message"].as_str()).unwrap_or("no detail");
        write!(f, "input is inconsistent ({} clash(es)): {first}", self.clashes.len())
    }
}
impl std::error::Error for Inconsistent {}
pub struct Inference {
    config: Config,
    reasoners: std::collections::BTreeMap<String, Arc<dyn Reasoner>>,
    gate: Mutex<()>,
    /// Every job, dependencies first.
    order: Vec<String>,
}
/// Which outside inputs, at which content, a rebuild read.
fn input_digests(fetched: &[Fetched]) -> Value {
    json!(fetched.iter().map(|f| [&f.name, &f.digest]).collect::<Vec<_>>())
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
/// A job whose `source_graphs` lists another job's output graph reads that job's deductions.
const DERIVED: &str = "urn:rdftool:derived:inference:";
fn upstream_name(graph: &str) -> Option<&str> {
    graph.strip_prefix(DERIVED)
}
/// Jobs this job reads directly.
fn depends_on(job: &Job) -> impl Iterator<Item = &str> {
    job.source_graphs.iter().filter_map(|graph| upstream_name(graph))
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
        if let Some(clashes) = result["clashes"].as_array().filter(|c| !c.is_empty()) {
            return Err(Inconsistent { clashes: clashes.clone() }.into());
        }
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
            rule_tests::validate(&job.tests)?;
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
                    !graph.starts_with("urn:rdftool:derived:") || upstream_name(graph).is_some(),
                    "only the output of an inference job can be a derived source graph"
                );
                if graph != "default" {
                    oxiri::Iri::parse(graph.as_str())?;
                }
            }
        }
        let order = dependency_order(&config.jobs)?;
        Ok(Self { config, reasoners, gate: Mutex::new(()), order })
    }
    /// A job whose last rebuild found its current input inconsistent reads as `inconsistent`, with
    /// the clashes, until the input or configuration changes or a rebuild succeeds.
    fn mark_inconsistent(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        fingerprint: &str,
        status: &mut Value,
    ) -> Result<()> {
        let Some(attempt) = status.as_object_mut().and_then(|s| s.remove("attempt")) else {
            return Ok(());
        };
        if status["state"] == "ready" || attempt["fingerprint"] != fingerprint {
            return Ok(());
        }
        let token = context.snapshot(&job.source_graphs)?.token;
        let inputs = if job.inputs.is_empty() {
            json!([])
        } else {
            match fetch_inputs(job) {
                Ok(now) => input_digests(&now),
                Err(_) => return Ok(()),
            }
        };
        if attempt["source_token"] == token.as_str() && attempt["inputs"] == inputs {
            status["state"] = json!("inconsistent");
            status["clashes"] = attempt["clashes"].clone();
            status["inconsistent_since"] = attempt["at"].clone();
        }
        Ok(())
    }
    /// Every job in rebuild order, with what it reads and what can be asked of it. No reasoner is
    /// called and nothing is read from the store.
    fn jobs(&self) -> Result<Value> {
        let jobs = self
            .order
            .iter()
            .filter_map(|name| self.job(name))
            .map(|job| {
                let (reasoner, _) = self.parts(job)?;
                Ok(json!({
                    "name": job.name, "provider": job.provider, "profile": job.profile,
                    "source_graphs": job.source_graphs, "depends_on": depends_on(job).collect::<Vec<_>>(),
                    "inputs": job.inputs.iter().map(|input| &input.name).collect::<Vec<_>>(),
                    "rules": job.rules.len(), "tests": job.tests.len(), "explains": reasoner.explains(),
                    "graph": format!("{DERIVED}{}", job.name),
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"jobs": jobs}))
    }
    fn job(&self, name: &str) -> Option<&Job> {
        self.config.jobs.iter().find(|job| job.name == name)
    }
    /// Jobs `name` reads from, directly or not, dependencies first.
    fn upstream(&self, name: &str) -> Vec<String> {
        let mut found = std::collections::HashSet::new();
        let mut stack = vec![name];
        while let Some(current) = stack.pop() {
            for dependency in self.job(current).into_iter().flat_map(depends_on) {
                if found.insert(dependency) {
                    stack.push(dependency);
                }
            }
        }
        self.order.iter().filter(|job| found.contains(job.as_str())).cloned().collect()
    }
    /// Jobs that read `name`, directly or not, dependencies first.
    fn downstream(&self, name: &str) -> Vec<String> {
        self.order
            .iter()
            .filter(|job| job.as_str() != name && self.upstream(job).iter().any(|up| up == name))
            .cloned()
            .collect()
    }
}
/// Orders jobs so each follows the jobs it reads; rejects unknown references and cycles.
fn dependency_order(jobs: &[Job]) -> Result<Vec<String>> {
    let names: std::collections::HashSet<&str> = jobs.iter().map(|job| job.name.as_str()).collect();
    for job in jobs {
        for dependency in depends_on(job) {
            ensure!(
                names.contains(dependency),
                "job {} reads the output of unknown job {dependency}",
                job.name
            );
        }
    }
    let mut order: Vec<String> = Vec::new();
    let mut remaining: Vec<&Job> = jobs.iter().collect();
    while !remaining.is_empty() {
        let (ready, blocked): (Vec<&Job>, Vec<&Job>) = remaining.into_iter().partition(|job| {
            depends_on(job).all(|dependency| order.iter().any(|d| d == dependency))
        });
        ensure!(
            !ready.is_empty(),
            "inference jobs form a cycle: {}",
            blocked.iter().map(|job| job.name.as_str()).collect::<Vec<_>>().join(", ")
        );
        order.extend(ready.into_iter().map(|job| job.name.clone()));
        remaining = blocked;
    }
    Ok(order)
}
fn fetch_inputs(job: &Job) -> Result<Vec<Fetched>> {
    job.inputs.iter().enumerate().map(|(index, input)| input.fetch(index)).collect()
}
impl Inference {
    fn parts(&self, job: &Job) -> Result<(&Arc<dyn Reasoner>, String)> {
        let provider = job.provider.as_ref().expect("validated provider");
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(job, &self.config.providers[provider]))?)
        );
        Ok((&self.reasoners[provider], fingerprint))
    }
    /// Rebuilds one job; the caller holds the gate.
    fn rebuild_one(&self, context: &dyn ReadContext, job: &Job) -> Result<Value> {
        let (reasoner, fingerprint) = self.parts(job)?;
        let source = context.snapshot(&job.source_graphs)?;
        let identity = reasoner.identity()?;
        ensure!(
            identity["profiles"]
                .as_array()
                .is_some_and(|profiles| profiles.iter().any(|profile| profile == &job.profile)),
            "reasoner does not support requested profile"
        );
        let fetched = fetch_inputs(job)?;
        let merged = inputs::merge(&source.ntriples, &fetched)?;
        let (triples, actual) = match reasoner.infer_rules(&job.profile, &merged, &job.rules) {
            Ok(done) => done,
            Err(error) => {
                let Some(inconsistent) = error.downcast_ref::<Inconsistent>() else {
                    return Err(error);
                };
                let kept = context.derived_status(&job.name)?["state"].clone();
                // Stored next to the derived graph, so it survives a restart; publishing clears it.
                context.record_attempt(
                    &job.name,
                    Some(json!({
                        "outcome": "inconsistent", "fingerprint": fingerprint,
                        "source_token": source.token, "inputs": input_digests(&fetched),
                        "clashes": inconsistent.clashes, "at": now(),
                    })),
                )?;
                return Ok(json!({
                    "job": job.name, "state": "inconsistent", "clashes": inconsistent.clashes,
                    "kept": kept, "note": "the derived graph was left as it was",
                }));
            }
        };
        ensure!(actual == identity, "reasoner identity changed during rebuild");
        let mut metadata =
            json!({"config_digest":fingerprint,"reasoner":identity,"profile":job.profile});
        if !fetched.is_empty() {
            metadata["inputs"] = fetched.iter().map(Fetched::record).collect();
        }
        let mut result = context.publish(&job.name, &source, &triples, metadata)?;
        result["state"] = json!("ready");
        Ok(result)
    }
    /// Rebuilds `job`, and with `cascade` the jobs around it that are no longer ready, in dependency
    /// order. Jobs that are still ready are skipped, so an unchanged result stops the cascade.
    fn rebuild_with_cascade(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        cascade: Option<&str>,
    ) -> Result<Value> {
        let upstream = if matches!(cascade, Some("upstream" | "both")) {
            self.upstream(&job.name)
        } else {
            vec![]
        };
        let downstream = if matches!(cascade, Some("downstream" | "both")) {
            self.downstream(&job.name)
        } else {
            vec![]
        };
        let mut steps = Vec::new();
        let mut ran = Vec::new();
        let not_ready = |name: &str| -> Result<bool> {
            let job = self.job(name).context("unknown inference job")?;
            Ok(self.status(context, job, &mut std::collections::BTreeMap::new())?["state"]
                != "ready")
        };
        let attempt = |name: &str, forced: bool| -> Result<Option<Value>> {
            if !forced && !not_ready(name)? {
                return Ok(None);
            }
            let target = self.job(name).context("unknown inference job")?;
            self.rebuild_one(context, target)
                .map(Some)
                .with_context(|| format!("rebuild of job {name} failed"))
        };
        let inconsistent = |outcome: &Option<Value>| {
            outcome.as_ref().is_some_and(|result| result["state"] == "inconsistent")
        };
        // After an inconsistent job, nothing that reads it runs: its output was not refreshed.
        let mut blocked_by: Option<String> = None;
        for name in &upstream {
            if blocked_by.is_some() {
                steps.push(json!({"job": name, "action": "blocked", "blocked_by": blocked_by}));
                continue;
            }
            let outcome = attempt(name, false)?;
            let action = if inconsistent(&outcome) {
                blocked_by = Some(name.clone());
                "inconsistent"
            } else if outcome.is_some() {
                ran.push(json!({"job": name}));
                "rebuilt"
            } else {
                "skipped"
            };
            steps.push(json!({"job": name, "action": action}));
        }
        let mut result = match &blocked_by {
            Some(name) => json!({
                "job": job.name, "state": "blocked", "blocked_by": name,
                "note": "an upstream job is inconsistent; fix it and rebuild again",
            }),
            None => attempt(&job.name, true)?.context("job was not rebuilt")?,
        };
        if result["state"] == "inconsistent" {
            blocked_by = Some(job.name.clone());
        }
        for name in &downstream {
            if blocked_by.is_some() {
                steps.push(json!({"job": name, "action": "blocked", "blocked_by": blocked_by}));
                continue;
            }
            let outcome = attempt(name, false)?;
            let action = if inconsistent(&outcome) {
                blocked_by = Some(name.clone());
                "inconsistent"
            } else if outcome.is_some() {
                ran.push(json!({"job": name}));
                "rebuilt"
            } else {
                "skipped"
            };
            steps.push(json!({"job": name, "action": action}));
        }
        if cascade.is_some() {
            result["cascade"] =
                json!({"upstream": upstream, "downstream": downstream, "steps": steps});
            // The engine reads `ran` to run follow-up rules written for the other jobs.
            result["ran"] = Value::Array(ran);
        }
        Ok(result)
    }
    /// The job's own state, then whether anything it reads is out of date. `memo` avoids asking twice.
    fn status(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        memo: &mut std::collections::BTreeMap<String, Value>,
    ) -> Result<Value> {
        if let Some(known) = memo.get(&job.name) {
            return Ok(known.clone());
        }
        let (reasoner, fingerprint) = self.parts(job)?;
        let mut status = context.derived_status(&job.name)?;
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
                                all.iter()
                                    .any(|r| r["name"] == input.name && r["digest"] == input.digest)
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
        // The store's token notices when an upstream graph's content changed; it cannot notice an
        // upstream job that is itself out of date and has not been rebuilt yet.
        let dependencies: Vec<&str> = depends_on(job).collect();
        let mut waiting = Vec::new();
        for dependency in &dependencies {
            let upstream = self.job(dependency).context("unknown inference job")?;
            if self.status(context, upstream, memo)?["state"] != "ready" {
                waiting.push(*dependency);
            }
        }
        if status["state"] == "ready" && !waiting.is_empty() {
            status["state"] = json!("stale");
        }
        if !waiting.is_empty() {
            status["stale_upstream"] = json!(waiting);
        }
        status["depends_on"] = json!(dependencies);
        self.mark_inconsistent(context, job, &fingerprint, &mut status)?;
        memo.insert(job.name.clone(), status.clone());
        Ok(status)
    }
    /// Runs a job's tests through its reasoner on the data each test supplies.
    fn test(&self, job: &Job) -> Result<Value> {
        let (reasoner, _) = self.parts(job)?;
        let results = job
            .tests
            .iter()
            .map(|test| {
                test.run(reasoner.as_ref(), &job.profile, &job.rules).or_else(|error| {
                    Ok::<_, anyhow::Error>(
                        json!({"name": test.name, "passed": false, "error": format!("{error:#}")}),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let passed = results.iter().all(|r| r["passed"] == true);
        Ok(json!({"job": job.name, "tests": results.len(), "passed": passed, "results": results}))
    }
    /// Every job that has tests; jobs without any are left out.
    fn test_all(&self) -> Result<Value> {
        let jobs = self
            .config
            .jobs
            .iter()
            .filter(|job| !job.tests.is_empty())
            .map(|job| self.test(job))
            .collect::<Result<Vec<_>>>()?;
        let passed = jobs.iter().all(|r| r["passed"] == true);
        Ok(json!({"passed": passed, "jobs": jobs}))
    }
    /// What the job reads and what reads it, with each one's state and the order to rebuild in.
    fn lineage(&self, context: &dyn ReadContext, job: &Job) -> Result<Value> {
        let mut memo = std::collections::BTreeMap::new();
        let mut describe = |names: Vec<String>| -> Result<Vec<Value>> {
            names
                .into_iter()
                .map(|name| {
                    let target = self.job(&name).context("unknown inference job")?;
                    let status = self.status(context, target, &mut memo)?;
                    Ok(json!({"job": name, "state": status["state"], "depends_on": status["depends_on"]}))
                })
                .collect()
        };
        let upstream = describe(self.upstream(&job.name))?;
        let downstream = describe(self.downstream(&job.name))?;
        let own = self.status(context, job, &mut std::collections::BTreeMap::new())?;
        let names = |entries: &[Value]| -> Vec<String> {
            entries.iter().filter_map(|entry| entry["job"].as_str().map(str::to_owned)).collect()
        };
        let mut order = names(&upstream);
        order.push(job.name.clone());
        order.extend(names(&downstream));
        Ok(json!({
            "job": job.name, "state": own["state"], "depends_on": own["depends_on"],
            "upstream": upstream, "downstream": downstream, "rebuild_order": order,
        }))
    }
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
            operations: vec![
                "rebuild".into(),
                "status".into(),
                "lineage".into(),
                "test".into(),
                "explain".into(),
                "jobs".into(),
            ],
            properties: vec![],
        }
    }
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        if operation == "explain" {
            return self.explain(context, &input);
        }
        if operation == "jobs" {
            ensure!(input.as_object().is_some_and(|o| o.is_empty()), "jobs takes no arguments");
            return self.jobs();
        }
        let object = input.as_object().context("expected object")?;
        let cascade = object.get("cascade").map(|value| value.as_str().unwrap_or_default());
        ensure!(
            (object.contains_key("job")
                || (matches!(operation, "test" | "status") && object.is_empty()))
                && object
                    .keys()
                    .all(|key| key == "job" || (key == "cascade" && operation == "rebuild")),
            "expected only a job argument (and cascade for rebuild)"
        );
        ensure!(
            matches!(cascade, None | Some("upstream" | "downstream" | "both")),
            "cascade must be upstream, downstream or both"
        );
        if operation == "test" && !object.contains_key("job") {
            return self.test_all();
        }
        if operation == "status" && !object.contains_key("job") {
            // Every job at once, dependencies first, sharing one memo so each is computed once.
            let mut memo = std::collections::BTreeMap::new();
            let mut all = Vec::new();
            for name in &self.order {
                let job = self.job(name).context("unknown inference job")?;
                let mut status = self.status(context, job, &mut memo)?;
                status["job"] = json!(name);
                all.push(status);
            }
            return Ok(json!({"jobs": all}));
        }
        let name = input["job"].as_str().context("job required")?;
        let job = self.job(name).context("unknown inference job")?;
        match operation {
            "test" => self.test(job),
            "rebuild" => {
                let _guard = self
                    .gate
                    .try_lock()
                    .map_err(|_| anyhow::anyhow!("inference rebuild already running"))?;
                self.rebuild_with_cascade(context, job, cascade)
            }
            "status" => self.status(context, job, &mut std::collections::BTreeMap::new()),
            "lineage" => self.lineage(context, job),
            _ => bail!("unknown inference operation"),
        }
    }
}
