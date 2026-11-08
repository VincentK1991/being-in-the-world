//! Application-registered adapters. Neither inference orchestration nor the host depends on these.
use anyhow::{Result, bail, ensure};
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

pub fn registry() -> Result<ReasonerRegistry> {
    let mut registry = ReasonerRegistry::default();
    registry.register("owlrl", HttpReasonerFactory)?;
    registry.register("jena-rules", HttpReasonerFactory)?;
    registry.register("sparql-rules", SparqlFactory)?;
    Ok(registry)
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
impl Reasoner for SparqlRules {
    fn identity(&self) -> Result<Value> {
        Ok(
            json!({"engine":"oxigraph","version":"0.5.13","adapter":env!("CARGO_PKG_VERSION"),"protocol":1,"profiles":["sparql-construct-once-v1"],"execution":"all-rules-read-original-snapshot"}),
        )
    }
    fn infer(&self, _: &str, _: &str) -> Result<(String, Value)> {
        bail!("SPARQL rules are required")
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
        let snapshot = Dataset::memory()?;
        // Import helpers rename blank nodes; snapshot replay must preserve identity.
        let mut transaction = snapshot.store().start_transaction()?;
        for (count, quad) in
            RdfParser::from_format(RdfFormat::NTriples).for_reader(ntriples.as_bytes()).enumerate()
        {
            ensure!(count < 100_000, "source exceeds 100000 triples");
            transaction.insert(&quad?);
        }
        transaction.commit()?;
        ensure!(snapshot.store().len()? <= 100_000, "source exceeds 100000 triples");
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
