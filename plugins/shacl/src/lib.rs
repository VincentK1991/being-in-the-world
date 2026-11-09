//! Explicit, dataset-scoped SHACL validation. No interception of ordinary writes.
use anyhow::{Context, Result, bail, ensure};
use rdf_plugin_api::{Facet, Manifest, Plugin, ReadContext, SDK_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read, sync::Arc, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub endpoint: String,
    pub jobs: Vec<Job>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub name: String,
    pub data_graphs: Vec<String>,
    pub shapes_graphs: Vec<String>,
    #[serde(default)]
    pub fail_on: Severity,
}
/// Gate threshold: Warning also rejects Violations; Info rejects all findings.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Violation,
    Warning,
    Info,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub severity: String,
    #[serde(rename = "focusNode")]
    pub focus_node: Vec<String>,
    #[serde(rename = "resultPath")]
    pub result_path: Vec<String>,
    pub value: Vec<String>,
    #[serde(rename = "sourceShape")]
    pub source_shape: Vec<String>,
    #[serde(rename = "sourceConstraintComponent")]
    pub constraint: Vec<String>,
    #[serde(rename = "resultMessage")]
    pub messages: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub conforms: bool,
    pub findings: Vec<Finding>,
    pub ntriples: String,
    pub engine_version: String,
}
pub trait Validator: Send + Sync {
    fn validate(&self, data: &str, shapes: &str) -> Result<Report>;
}
struct Jena {
    endpoint: String,
    client: reqwest::blocking::Client,
}
impl Validator for Jena {
    fn validate(&self, data: &str, shapes: &str) -> Result<Report> {
        let response = self
            .client
            .post(format!("{}/validate", self.endpoint))
            .json(&json!({"data":data,"shapes":shapes}))
            .send()?;
        let status = response.status();
        let mut bytes = Vec::new();
        response.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 16 * 1024 * 1024, "validation response exceeds 16 MiB");
        ensure!(
            status.is_success(),
            "SHACL provider {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let report: Report = serde_json::from_slice(&bytes)?;
        ensure!(report.engine_version == "6.2.0", "unsupported Jena version");
        Ok(report)
    }
}
pub struct Shacl {
    config: Config,
    validator: Arc<dyn Validator>,
}
fn fingerprint(job: &Job) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(job)?)))
}
impl Shacl {
    pub fn new(config: Config) -> Result<Self> {
        let url = reqwest::Url::parse(&config.endpoint)?;
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "SHACL endpoint must be an explicit loopback HTTP origin"
        );
        let validator = Arc::new(Jena {
            endpoint: config.endpoint.trim_end_matches('/').into(),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(70))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?,
        });
        Self::with_validator(config, validator)
    }
    pub fn with_validator(config: Config, validator: Arc<dyn Validator>) -> Result<Self> {
        ensure!(
            !config.jobs.is_empty() && config.jobs.len() <= 64,
            "configure 1 to 64 validation jobs"
        );
        let mut names = BTreeSet::new();
        for job in &config.jobs {
            ensure!(
                !job.name.is_empty()
                    && job.name.len() <= 64
                    && job
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    && names.insert(job.name.clone()),
                "invalid or duplicate SHACL job name"
            );
            ensure!(
                !job.data_graphs.is_empty()
                    && !job.shapes_graphs.is_empty()
                    && job.data_graphs.len() + job.shapes_graphs.len() <= 64,
                "select data and shapes graphs (at most 64 total)"
            );
            let mut graphs = BTreeSet::new();
            for graph in job.data_graphs.iter().chain(&job.shapes_graphs) {
                ensure!(
                    !graph.starts_with("urn:rdftool:derived:shacl:"),
                    "a validation report cannot be a source"
                );
                if graph != "default" {
                    oxiri::Iri::parse(graph.as_str())?;
                }
                ensure!(graphs.insert(graph), "duplicate or overlapping data/shapes graph");
            }
        }
        Ok(Self { config, validator })
    }
    fn run(&self, context: &dyn ReadContext, job: &Job, check: bool) -> Result<Value> {
        let graphs: Vec<_> = job.data_graphs.iter().chain(&job.shapes_graphs).cloned().collect();
        // SDK v3 snapshots merge graphs. Bracket the two role-specific snapshots with
        // the complete source token; publication checks that token again under the host write gate.
        let source = context.snapshot(&graphs)?;
        let data = context.snapshot(&job.data_graphs)?;
        let shapes = context.snapshot(&job.shapes_graphs)?;
        ensure!(
            context.snapshot(&graphs)?.token == source.token,
            "validation sources changed; retry"
        );
        ensure!(
            !shapes.ntriples.is_empty(),
            "shapes graph is empty; upload shapes before validating"
        );
        let report = self.validator.validate(&data.ntriples, &shapes.ntriples)?;
        ensure!(
            report.findings.len() <= 1000 && report.ntriples.len() <= 16 * 1024 * 1024,
            "validation report exceeds limits"
        );
        ensure!(report.conforms == report.findings.is_empty(), "inconsistent SHACL report");
        let (mut violations, mut warnings, mut info) = (0, 0, 0);
        for finding in &report.findings {
            match finding.severity.as_str() {
                "Violation" => violations += 1,
                "Warning" => warnings += 1,
                "Info" => info += 1,
                _ => bail!("unsupported SHACL result severity"),
            }
        }
        let gate_passed = violations == 0
            && match job.fail_on {
                Severity::Violation => true,
                Severity::Warning => warnings == 0,
                Severity::Info => warnings == 0 && info == 0,
            };
        let summary = json!({"conforms":report.conforms,"gate_passed":gate_passed,
            "counts":{"violation":violations,"warning":warnings,"info":info},
            "fail_on":job.fail_on,"config_digest":fingerprint(job)?,
            "engine":"jena","engine_version":report.engine_version,"profile":"shacl-core-v1"});
        let published = context.publish(&job.name, &source, &report.ntriples, summary.clone())?;
        if check && !gate_passed {
            bail!(
                "SHACL gate failed: {violations} violation(s), {warnings} warning(s), {info} info; report saved in {}. No user data was changed.",
                published["graph"]
            );
        }
        Ok(json!({"job":job.name,"mode":if check {"check"} else {"report-only"},
            "summary":summary,"findings":report.findings,"publication":published}))
    }
}
impl Plugin for Shacl {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "shacl".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["jobs".into(), "validate".into(), "check".into(), "status".into()],
            properties: vec![],
        }
    }
    fn facets(&self) -> Vec<Facet> {
        self.config.jobs.iter().map(|job| Facet::new("validation-job", &job.name)).collect()
    }
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            job: String,
        }
        if operation == "jobs" {
            ensure!(
                input.as_object().is_some_and(|o| o.is_empty()),
                "jobs expects an empty object"
            );
            return Ok(
                json!({"jobs":self.config.jobs,"profile":"shacl-core-v1","automatic_write_enforcement":false}),
            );
        }
        ensure!(
            matches!(operation, "validate" | "check" | "status"),
            "unsupported SHACL operation"
        );
        let input: Input = serde_json::from_value(input)?;
        let job = self
            .config
            .jobs
            .iter()
            .find(|j| j.name == input.job)
            .context("unknown validation job")?;
        if operation == "status" {
            let mut status = context.derived_status(&job.name)?;
            if status["state"] == "ready"
                && status["metadata"]["config_digest"] != fingerprint(job)?
            {
                status["state"] = json!("stale");
            }
            return Ok(status);
        }
        self.run(context, job, operation == "check")
    }
}
