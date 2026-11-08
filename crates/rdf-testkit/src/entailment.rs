//! Entailment checks through the product's reasoner adapters (the same HTTP clients inference
//! jobs use), so conformance measures the deployed reasoning path, not a library in isolation.
use anyhow::{Context, Result, bail};
use rdf_plugin_inference::{HttpReasonerFactory, Reasoner, ReasonerFactory};
use rdf_store::{
    engine::Dataset,
    oxigraph::{
        io::{RdfFormat, RdfParser, RdfSerializer},
        model::*,
        sparql::{QueryResults, SparqlEvaluator},
    },
};
use serde_json::json;
use std::sync::Arc;

/// What a test asks the reasoner to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    Rdfs,
    /// OWL 2 RL rules: a sound, partial implementation of the OWL 2 RDF-based semantics.
    OwlRl,
}
enum Kind {
    /// The OWL-RL sidecar: profiles `rdfs-v1` and `owlrl-core-v1`.
    Owlrl,
    /// The Jena forward-rule sidecar given the RDFS rules in `rdfs.rules`.
    JenaRdfs(Vec<String>),
}
pub struct Reasoning {
    reasoner: Arc<dyn Reasoner>,
    kind: Kind,
}
/// The outcome of asking for a closure.
pub enum Derivation {
    /// Triples the reasoner added, as N-Triples.
    Derived(String),
    /// The reasoner found the input inconsistent.
    Inconsistent(String),
    /// The input is outside the profile the reasoner declares (an unsupported term or shape).
    Rejected(String),
}
impl Reasoning {
    /// `owlrl=<url>` or `jena-rdfs=<url>`.
    pub fn parse(spec: &str) -> Result<Self> {
        let (name, endpoint) = spec.split_once('=').context("expected name=url")?;
        let reasoner = HttpReasonerFactory.create(&json!({ "endpoint": endpoint }))?;
        let kind = match name {
            "owlrl" => Kind::Owlrl,
            "jena-rdfs" => Kind::JenaRdfs(vec![include_str!("rdfs.rules").to_owned()]),
            other => bail!("unknown reasoner {other}: expected owlrl or jena-rdfs"),
        };
        Ok(Self { reasoner, kind })
    }
    fn profile(&self, regime: Regime) -> Option<(&'static str, &[String])> {
        match (&self.kind, regime) {
            (Kind::Owlrl, Regime::Rdfs) => Some(("rdfs-v1", &[])),
            (Kind::Owlrl, Regime::OwlRl) => Some(("owlrl-core-v1", &[])),
            (Kind::JenaRdfs(rules), Regime::Rdfs) => Some(("jena-forward-v1", rules)),
            (Kind::JenaRdfs(_), Regime::OwlRl) => None,
        }
    }
    pub fn supports(&self, regime: Regime) -> bool {
        self.profile(regime).is_some()
    }
    pub fn derive(&self, regime: Regime, ntriples: &str) -> Result<Derivation> {
        let (profile, rules) =
            self.profile(regime).context("regime not offered by this reasoner")?;
        match self.reasoner.infer_rules(profile, ntriples, rules) {
            Ok((derived, _)) => Ok(Derivation::Derived(
                // Generalised triples (a literal subject from rdfs3 on a literal) are not RDF.
                derived.lines().filter(|l| !l.starts_with('"')).collect::<Vec<_>>().join("\n"),
            )),
            Err(error) => {
                let message = format!("{error:#}");
                if message.contains("inconsisten") {
                    Ok(Derivation::Inconsistent(message))
                } else if ["unsupported", "must be named", "exceeds", "only "]
                    .iter()
                    .any(|m| message.contains(m))
                {
                    Ok(Derivation::Rejected(message))
                } else {
                    Err(error)
                }
            }
        }
    }
}
pub fn ntriples(quads: impl IntoIterator<Item = Quad>) -> Result<String> {
    let mut serializer = RdfSerializer::from_format(RdfFormat::NTriples).for_writer(Vec::new());
    for quad in quads {
        serializer.serialize_triple(&Triple::from(quad))?;
    }
    Ok(String::from_utf8(serializer.finish()?)?)
}
pub fn parse(text: &str, format: RdfFormat, base: &str) -> Result<Vec<Quad>> {
    Ok(RdfParser::from_format(format)
        .with_base_iri(base)?
        .for_reader(text.as_bytes())
        .collect::<Result<Vec<_>, _>>()?)
}
/// Simple entailment: does `graph` contain an instance of `conclusion`, blank nodes in the
/// conclusion read as existential variables? On failure, the conclusion triples that have no
/// instance even on their own, for triage (empty when only the combination fails).
pub fn entails(graph: &[Quad], conclusion: &[Quad]) -> Result<Result<(), Vec<String>>> {
    let data = Dataset::memory()?;
    for quad in graph {
        data.store().insert(&Quad::new(
            quad.subject.clone(),
            quad.predicate.clone(),
            quad.object.clone(),
            GraphName::DefaultGraph,
        ))?;
    }
    if ask(&data, conclusion)? {
        return Ok(Ok(()));
    }
    let mut unmatched = Vec::new();
    for quad in conclusion {
        if !ask(&data, std::slice::from_ref(quad))? {
            unmatched.push(Triple::from(quad.clone()).to_string());
        }
    }
    Ok(Err(unmatched))
}
fn ask(data: &Dataset, patterns: &[Quad]) -> Result<bool> {
    let term = |t: &Term| match t {
        Term::BlankNode(b) => {
            format!("?b_{}", b.as_str().replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
        }
        other => other.to_string(),
    };
    let patterns = patterns
        .iter()
        .map(|q| {
            format!("{} {} {} .", term(&q.subject.clone().into()), q.predicate, term(&q.object))
        })
        .collect::<Vec<_>>()
        .join("\n");
    match SparqlEvaluator::new()
        .parse_query(&format!("ASK {{ {patterns} }}"))?
        .on_store(data.store())
        .execute()?
    {
        QueryResults::Boolean(value) => Ok(value),
        _ => bail!("ASK returned no boolean"),
    }
}
