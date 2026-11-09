//! Application-registered adapters. Neither inference orchestration nor the host depends on these.
use anyhow::{Context, Result, bail, ensure};
use rdf_plugin_inference::{HttpReasonerFactory, Reasoner, ReasonerFactory, ReasonerRegistry};
use rdf_store::{
    engine::Dataset,
    oxigraph::{
        io::{RdfFormat, RdfParser},
        model::{GraphName, Quad},
        sparql::{CancellationToken, QueryResults, SparqlEvaluator},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

mod diagnostics;
mod explain;
pub mod owl2rl;

pub fn registry() -> Result<ReasonerRegistry> {
    let mut registry = ReasonerRegistry::default();
    registry.register("owlrl", HttpReasonerFactory)?;
    registry.register("jena-rules", HttpReasonerFactory)?;
    registry.register("sparql-rules", SparqlFactory)?;
    registry.register("owlrl-native", NativeFactory)?;
    Ok(registry)
}
/// OWL 2 RL evaluated in process (`owl2rl.rs`): no sidecar, and every deduction can be explained.
pub struct NativeFactory;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeConfig {
    #[serde(default = "timeout")]
    cancel_after_seconds: u64,
    #[serde(default = "limit")]
    max_triples: usize,
}
pub struct NativeOwlRl(owl2rl::Limits);
impl NativeOwlRl {
    pub const PROFILE: &'static str = "owl2rl-v1";
    pub fn new(cancel_after_seconds: u64, max_triples: usize) -> Self {
        Self(owl2rl::Limits {
            max_derived: max_triples,
            timeout: Duration::from_secs(cancel_after_seconds),
        })
    }
}
impl ReasonerFactory for NativeFactory {
    fn create(&self, config: &Value) -> Result<Arc<dyn Reasoner>> {
        let config: NativeConfig = serde_json::from_value(config.clone())?;
        ensure!(
            (1..=60).contains(&config.cancel_after_seconds)
                && (1..=200_000).contains(&config.max_triples),
            "invalid OWL 2 RL limits"
        );
        Ok(Arc::new(NativeOwlRl::new(config.cancel_after_seconds, config.max_triples)))
    }
}
impl Reasoner for NativeOwlRl {
    fn diagnostics(&self, _: &str, data: &str) -> Result<Value> {
        diagnostics::inspect(data)
    }
    fn identity(&self) -> Result<Value> {
        Ok(json!({"engine": "rdftool-owl2rl", "adapter": env!("CARGO_PKG_VERSION"), "protocol": 2,
                  "profiles": [Self::PROFILE], "rules": "OWL 2 RL/RDF tables 4-7 and 9; no datatype rules",
                  "axiomatic_triples": "scoped", "output_policy": 2, "rules_revision": 4, "justifications": true, "datatype_checks":"partial", "profile_complete":false, "proof_policy":"first-justification"}))
    }
    fn infer(&self, profile: &str, ntriples: &str) -> Result<(String, Value)> {
        ensure!(profile == Self::PROFILE, "unsupported OWL 2 RL profile");
        ensure!(ntriples.len() <= 16 * 1024 * 1024, "source exceeds 16 MiB");
        let (derived, _) = owl2rl::infer(ntriples, &self.0)?;
        Ok((derived, self.identity()?))
    }
    fn explains(&self) -> bool {
        true
    }
    fn explain(
        &self,
        profile: &str,
        ntriples: &str,
        rules: &[String],
        target: &str,
    ) -> Result<Option<Vec<Value>>> {
        ensure!(profile == Self::PROFILE && rules.is_empty(), "unsupported OWL 2 RL profile");
        owl2rl::explain(ntriples, &self.0, target).map(Some)
    }
}
pub struct SparqlFactory;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default = "timeout")]
    cancel_after_seconds: u64,
    #[serde(default = "limit")]
    max_triples: usize,
}
fn timeout() -> u64 {
    30
}
fn limit() -> usize {
    200_000
}
struct SparqlRules(Config);
impl ReasonerFactory for SparqlFactory {
    fn create(&self, config: &Value) -> Result<Arc<dyn Reasoner>> {
        let config: Config = serde_json::from_value(config.clone())?;
        ensure!(
            (1..=60).contains(&config.cancel_after_seconds)
                && (1..=200_000).contains(&config.max_triples),
            "invalid SPARQL rule limits"
        );
        Ok(Arc::new(SparqlRules(config)))
    }
}
/// An in-memory copy of the rule input. Import helpers rename blank nodes; replay must preserve identity.
fn load(ntriples: &str) -> Result<Dataset> {
    let snapshot = Dataset::memory()?;
    let mut transaction = snapshot.store().start_transaction()?;
    for (count, quad) in
        RdfParser::from_format(RdfFormat::NTriples).for_reader(ntriples.as_bytes()).enumerate()
    {
        ensure!(count < 100_000, "source exceeds 100000 triples");
        transaction.insert(&quad?);
    }
    transaction.commit()?;
    ensure!(snapshot.store().len()? <= 100_000, "source exceeds 100000 triples");
    Ok(snapshot)
}
impl Reasoner for SparqlRules {
    fn identity(&self) -> Result<Value> {
        Ok(
            json!({"engine":"oxigraph","version":"0.5.13","adapter":env!("CARGO_PKG_VERSION"),"protocol":1,"profiles":["sparql-construct-once-v1"],"execution":"all-rules-read-original-snapshot"}),
        )
    }
    fn infer(&self, _: &str, _: &str) -> Result<(String, Value)> {
        bail!("SPARQL rules are required")
    }
    fn explains(&self) -> bool {
        true
    }
    fn explain(
        &self,
        profile: &str,
        ntriples: &str,
        rules: &[String],
        target: &str,
    ) -> Result<Option<Vec<Value>>> {
        ensure!(profile == "sparql-construct-once-v1", "unsupported SPARQL rule profile");
        let parsed =
            rules.iter().map(|rule| Dataset::prepare_query(rule)).collect::<Result<Vec<_>>>()?;
        let mut quads = RdfParser::from_format(RdfFormat::NTriples).for_reader(target.as_bytes());
        let quad = quads.next().context("no triple to explain")??;
        ensure!(quads.next().is_none(), "explain one triple at a time");
        let triple =
            rdf_store::oxigraph::model::Triple::new(quad.subject, quad.predicate, quad.object);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (_done, receiver) = std::sync::mpsc::channel::<()>();
        let duration = Duration::from_secs(self.0.cancel_after_seconds);
        std::thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(duration),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                token.cancel();
            }
        });
        explain::explain(&load(ntriples)?, &parsed, rules, &triple, &cancel).map(Some)
    }
    fn infer_rules(
        &self,
        profile: &str,
        ntriples: &str,
        rules: &[String],
    ) -> Result<(String, Value)> {
        ensure!(profile == "sparql-construct-once-v1", "unsupported SPARQL rule profile");
        ensure!(!rules.is_empty() && rules.len() <= 64, "provide 1 to 64 CONSTRUCT rules");
        ensure!(
            ntriples.len() <= 16 * 1024 * 1024
                && rules.iter().map(String::len).sum::<usize>() <= 1024 * 1024,
            "input exceeds rule limits"
        );
        // Reuse the adapter's parsed SERVICE policy, including nested/SILENT forms.
        let parsed = rules
            .iter()
            .map(|rule| {
                let query = Dataset::prepare_query(rule)?;
                ensure!(
                    matches!(&query, spargebra::Query::Construct { dataset: None, .. }),
                    "rules must be CONSTRUCT without FROM/FROM NAMED"
                );
                Ok(query)
            })
            .collect::<Result<Vec<_>>>()?;
        let snapshot = load(ntriples)?;
        let cancel = CancellationToken::new();
        let worker_token = cancel.clone();
        let (done, receiver) = std::sync::mpsc::channel::<()>();
        let duration = Duration::from_secs(self.0.cancel_after_seconds);
        std::thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(duration),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                worker_token.cancel();
            }
        });
        // Dropping sender on every return disconnects and terminates the timer thread.
        let _done = done;
        let mut output = BTreeSet::new();
        let mut bytes = 0;
        for query in parsed {
            let results = SparqlEvaluator::new()
                .with_cancellation_token(cancel.clone())
                .for_query(query)
                .on_store(snapshot.store())
                .execute()?;
            let QueryResults::Graph(triples) = results else { bail!("expected CONSTRUCT results") };
            for triple in triples {
                ensure!(!cancel.is_cancelled(), "SPARQL rule cancelled");
                let triple = triple?;
                let quad = Quad::new(
                    triple.subject.clone(),
                    triple.predicate.clone(),
                    triple.object.clone(),
                    GraphName::DefaultGraph,
                );
                if snapshot.store().contains(&quad)? {
                    continue;
                }
                let line = format!("{triple} .\n");
                if output.insert(line.clone()) {
                    bytes += line.len();
                }
                ensure!(
                    output.len() <= self.0.max_triples && bytes <= 16 * 1024 * 1024,
                    "derived SPARQL output exceeds limit"
                );
            }
        }
        ensure!(!cancel.is_cancelled(), "SPARQL rule cancelled");
        Ok((output.into_iter().collect(), self.identity()?))
    }
}
