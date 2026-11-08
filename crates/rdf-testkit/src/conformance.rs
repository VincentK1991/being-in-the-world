//! RDF and SPARQL manifest interpretation. Unsupported cases are never counted as passes.
//!
//! Inside a checkout of the W3C `rdf-tests` repository (found by its `test-map.json`), every file
//! is addressed by its published IRI under `https://w3c.github.io/rdf-tests/`, so manifests, data,
//! graph names and query bases agree with the expected results. Elsewhere files keep `file:` IRIs.
use crate::entailment::{self, Derivation, Reasoning, Regime};
use anyhow::{Context, Result, bail, ensure};
use rdf_store::{
    comparison,
    engine::{Dataset, canonical_quads, format_for_path},
    oxigraph::{
        io::{RdfFormat, RdfParser},
        model::*,
        sparql::{
            QueryResults, SparqlEvaluator,
            results::{QueryResultsFormat, QueryResultsSerializer},
        },
        store::Store,
    },
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const QT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-query#";
const UT: &str = "http://www.w3.org/2009/sparql/tests/test-update#";
const RS: &str = "http://www.w3.org/2001/sw/DataAccess/tests/result-set#";
const SD: &str = "http://www.w3.org/ns/sparql-service-description#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const ENT: &str = "http://www.w3.org/ns/entailment/";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const OWLTEST: &str = "http://www.w3.org/2007/OWL/testOntology#";
const PUBLISHED: &str = "https://w3c.github.io/rdf-tests/";
/// Which query path evaluates the cases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Oxigraph directly: measures the upstream engine.
    #[default]
    Oxigraph,
    /// `rdf_engine::Database` with the default plugins: our parser, policy and property-function
    /// rewrite in front of Oxigraph. Regressions in our own layer show up here.
    Database,
}
#[derive(Serialize, Default)]
pub struct Report {
    pub passed: usize,
    pub failed: usize,
    pub unsupported: usize,
    pub skipped: usize,
    /// Results equal only after numeric/boolean values are compared by value: the engine changed
    /// a lexical form (Oxigraph stores `"01"^^xsd:integer` as `1`). Not counted as passed.
    pub value_only: usize,
    pub cases: Vec<Case>,
}
#[derive(Serialize)]
pub struct Case {
    pub id: String,
    pub status: String,
    pub detail: String,
}
enum Outcome {
    Passed,
    ValueOnly(String),
    Unsupported(String),
}
pub fn run(path: &Path) -> Result<Report> {
    run_with(path, Engine::Oxigraph)
}
pub fn run_with(path: &Path, engine: Engine) -> Result<Report> {
    run_reasoning(path, engine, None)
}
/// With a reasoner, entailment cases (RDF 1.1 Semantics `rdf-mt`, SPARQL 1.1 entailment regimes,
/// OWL 2 test-case exports) run through it instead of being reported unsupported.
pub fn run_reasoning(path: &Path, engine: Engine, reasoning: Option<Reasoning>) -> Result<Report> {
    let canonical = path.canonicalize()?;
    let root =
        canonical.ancestors().find(|p| p.join("test-map.json").is_file()).map(Path::to_path_buf);
    let suite = Suite { root, engine, reasoning };
    let mut report = Report::default();
    suite.visit(&canonical, &mut HashSet::new(), &mut report)?;
    Ok(report)
}
/// Oxigraph's RDF/XML parser accepts only double-quoted `<!ENTITY>` values; XML allows either.
fn quoted_entities(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("<!ENTITY") && !trimmed.contains('"') {
                line.replace('\'', "\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn record(report: &mut Report, id: String, outcome: Result<Outcome>) {
    let (status, detail) = match outcome {
        Ok(Outcome::Passed) => {
            report.passed += 1;
            ("passed", String::new())
        }
        Ok(Outcome::ValueOnly(diff)) => {
            report.value_only += 1;
            ("value-only", diff)
        }
        Ok(Outcome::Unsupported(reason)) => {
            report.unsupported += 1;
            ("unsupported", reason)
        }
        Err(error) => {
            report.failed += 1;
            ("failed", format!("{error:#}"))
        }
    };
    report.cases.push(Case { id, status: status.into(), detail });
}
fn objects(data: &Dataset, subject: &Term, predicate: &str) -> Result<Vec<Term>> {
    let subject = match subject {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
        Term::BlankNode(n) => NamedOrBlankNode::BlankNode(n.clone()),
        _ => bail!("literal manifest subject"),
    };
    Ok(data
        .store()
        .quads_for_pattern(
            Some(subject.as_ref()),
            Some(NamedNode::new(predicate)?.as_ref()),
            None,
            None,
        )
        .map(|q| q.map(|q| q.object))
        .collect::<Result<Vec<_>, _>>()?)
}
fn one(data: &Dataset, subject: &Term, predicate: &str) -> Result<Term> {
    objects(data, subject, predicate)?
        .into_iter()
        .next()
        .with_context(|| format!("missing manifest property <{predicate}>"))
}
fn list(data: &Dataset, mut head: Term) -> Result<Vec<Term>> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    while head.to_string() != format!("<{RDF}nil>") {
        ensure!(seen.insert(head.clone()), "cyclic RDF list");
        values.push(one(data, &head, &format!("{RDF}first"))?);
        head = one(data, &head, &format!("{RDF}rest"))?;
    }
    Ok(values)
}
/// A term or an RDF list of terms, as manifests write `sd:entailmentRegime` either way.
fn terms(data: &Dataset, term: Term) -> Result<Vec<Term>> {
    if matches!(term, Term::BlankNode(_)) || term.to_string() == format!("<{RDF}nil>") {
        list(data, term)
    } else {
        Ok(vec![term])
    }
}
fn format(path: &Path) -> Result<RdfFormat> {
    match path.extension().and_then(|v| v.to_str()) {
        Some("rdf") => Ok(RdfFormat::RdfXml),
        _ => format_for_path(path),
    }
}
fn quads(store: &Store) -> Result<String> {
    Ok(canonical_quads(store.iter().collect::<Result<Vec<_>, _>>()?))
}
/// Lines only in one canonical form, a few of each, for triage.
fn diff(expected: &str, actual: &str) -> String {
    let e: BTreeSet<&str> = expected.lines().collect();
    let a: BTreeSet<&str> = actual.lines().collect();
    let only = |x: &BTreeSet<&str>, y: &BTreeSet<&str>| {
        x.difference(y).take(4).copied().collect::<Vec<_>>().join(" | ")
    };
    format!("missing: {} ; unexpected: {}", only(&e, &a), only(&a, &e))
}
/// IRIs a query names in FROM / FROM NAMED; harnesses load them, the engine never fetches.
fn dataset_clauses(query: &str) -> Result<Vec<NamedNode>> {
    use spargebra::Query;
    let dataset = match spargebra::SparqlParser::new().parse_query(query)? {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset,
    };
    Ok(dataset
        .map(|d| d.default.into_iter().chain(d.named.unwrap_or_default()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|n| NamedNode::new_unchecked(n.into_string()))
        .collect())
}
/// The plugin set a dataset gets when the server starts without sidecar configuration.
pub fn database(dataset: Dataset) -> Result<rdf_engine::Database> {
    let mut database = rdf_engine::Database::from_dataset(dataset);
    database.register(rdf_plugin_dataset_summary::DatasetSummary)?;
    database.register(rdf_plugin_custom_properties::CustomProperties::examples())?;
    database.register(rdf_plugin_catalog::PluginCatalog::new(database.catalog()))?;
    Ok(database)
}
struct Suite {
    root: Option<PathBuf>,
    engine: Engine,
    reasoning: Option<Reasoning>,
}
impl Suite {
    fn iri(&self, path: &Path) -> Result<String> {
        let canonical = path.canonicalize()?;
        if let Some(rest) = self.root.as_ref().and_then(|r| canonical.strip_prefix(r).ok()) {
            return Ok(url::Url::parse(PUBLISHED)?.join(&rest.to_string_lossy())?.to_string());
        }
        Ok(url::Url::from_file_path(&canonical)
            .map_err(|_| anyhow::anyhow!("invalid path"))?
            .to_string())
    }
    fn local(&self, term: &Term) -> Result<PathBuf> {
        let Term::NamedNode(node) = term else { bail!("expected file IRI") };
        if let (Some(root), Some(rest)) = (&self.root, node.as_str().strip_prefix(PUBLISHED)) {
            return Ok(root.join(rest));
        }
        url::Url::parse(node.as_str())?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("only local fixture URLs are allowed: {node}"))
    }
    fn load(&self, data: &Dataset, path: &Path, graph: GraphName) -> Result<()> {
        let parser = RdfParser::from_format(format(path)?)
            .with_base_iri(&self.iri(path)?)?
            .with_default_graph(graph);
        let mut text = fs::read_to_string(path)?;
        if format(path)? == RdfFormat::RdfXml {
            text = quoted_entities(&text);
        }
        data.store().load_from_reader(parser, text.as_bytes())?;
        Ok(())
    }
    fn visit(&self, path: &Path, seen: &mut HashSet<PathBuf>, report: &mut Report) -> Result<()> {
        let path = path.canonicalize()?;
        if !seen.insert(path.clone()) {
            return Ok(());
        }
        let manifest = Dataset::memory()?;
        self.load(&manifest, &path, GraphName::DefaultGraph)?;
        let mut entries = Vec::new();
        let mut includes = Vec::new();
        for quad in manifest.store().iter() {
            let q = quad?;
            if q.predicate.as_str() == format!("{MF}entries") {
                entries.extend(list(&manifest, q.object)?);
            } else if q.predicate.as_str() == format!("{MF}include") {
                includes.extend(list(&manifest, q.object)?);
            }
        }
        if entries.is_empty() && includes.is_empty() {
            return self.owl2(&manifest, report);
        }
        for entry in entries {
            let kind = one(&manifest, &entry, &format!("{RDF}type"))?.to_string();
            let outcome = self.case(&manifest, &entry, &kind);
            record(report, entry.to_string(), outcome);
        }
        for include in includes {
            self.visit(&self.local(&include)?, seen, report)?;
        }
        Ok(())
    }
    fn dataset_description(&self, manifest: &Dataset, node: &Term, ns: &str) -> Result<Dataset> {
        let data = Dataset::memory()?;
        for file in objects(manifest, node, &format!("{ns}data"))? {
            self.load(&data, &self.local(&file)?, GraphName::DefaultGraph)?;
        }
        for graph_data in objects(manifest, node, &format!("{ns}graphData"))? {
            if let Term::NamedNode(name) = &graph_data {
                self.load(&data, &self.local(&graph_data)?, GraphName::NamedNode(name.clone()))?;
            } else {
                let file = one(manifest, &graph_data, &format!("{ns}graph"))?;
                let name =
                    one(manifest, &graph_data, "http://www.w3.org/2000/01/rdf-schema#label")?;
                let name = match name {
                    Term::NamedNode(n) => n,
                    Term::Literal(l) => NamedNode::new(l.value())?,
                    _ => bail!("invalid graph name"),
                };
                data.store().insert_named_graph(&name)?;
                self.load(&data, &self.local(&file)?, GraphName::NamedNode(name))?;
            }
        }
        Ok(data)
    }
    fn parses_query(&self, query: &str, base: &str) -> Result<bool> {
        Ok(match self.engine {
            Engine::Oxigraph => {
                SparqlEvaluator::new().with_base_iri(base)?.parse_query(query).is_ok()
            }
            Engine::Database => {
                spargebra::SparqlParser::new().with_base_iri(base)?.parse_query(query).is_ok()
            }
        })
    }
    fn parses_update(&self, update: &str, base: &str) -> Result<bool> {
        Ok(match self.engine {
            Engine::Oxigraph => {
                SparqlEvaluator::new().with_base_iri(base)?.parse_update(update).is_ok()
            }
            Engine::Database => {
                spargebra::SparqlParser::new().with_base_iri(base)?.parse_update(update).is_ok()
            }
        })
    }
    /// Query text with the test's base made explicit, so every consumer resolves IRIs alike.
    fn text(&self, path: &Path) -> Result<String> {
        Ok(format!("BASE <{}>\n{}", self.iri(path)?, fs::read_to_string(path)?))
    }
    fn case(&self, manifest: &Dataset, entry: &Term, kind: &str) -> Result<Outcome> {
        for (name, why) in [
            ("ProtocolTest", "needs an HTTP protocol runner"),
            ("GraphStoreProtocolTest", "needs an HTTP graph store runner"),
            ("ServiceDescriptionTest", "needs an HTTP service description runner"),
        ] {
            if kind.ends_with(&format!("#{name}>")) {
                return Ok(Outcome::Unsupported(format!("{name}: {why}")));
            }
        }
        let action = one(manifest, entry, &format!("{MF}action"))?;
        if kind.contains("SyntaxTest")
            || kind.contains("PositiveSyntax")
            || kind.contains("NegativeSyntax")
            || kind.contains("NegativeEval")
        {
            let file = self.local(&action)?;
            let input = fs::read_to_string(&file)?;
            let negative = kind.contains("Negative");
            let base = self.iri(&file)?;
            let valid = match file.extension().and_then(|e| e.to_str()) {
                Some("rq") => self.parses_query(&input, &base)?,
                Some("ru") => self.parses_update(&input, &base)?,
                _ => RdfParser::from_format(format(&file)?)
                    .with_base_iri(&base)?
                    .for_reader(input.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .is_ok(),
            };
            ensure!(valid != negative, "syntax expectation mismatch");
            return Ok(Outcome::Passed);
        }
        if kind.contains("QueryEvaluationTest") || kind.contains("CSVResultFormatTest") {
            let data = self.dataset_description(manifest, &action, QT)?;
            if let Ok(regime) = one(manifest, &action, &format!("{SD}entailmentRegime")) {
                let regimes =
                    terms(manifest, regime)?.iter().map(Term::to_string).collect::<Vec<_>>();
                let has = |name: &str| regimes.iter().any(|r| r == &format!("<{ENT}{name}>"));
                // The expected answer holds under every listed regime: use the first one offered.
                let listed: Vec<Regime> = [("RDFS", Regime::Rdfs), ("OWL-RDF-Based", Regime::OwlRl)]
                    .into_iter()
                    .filter(|(name, _)| has(name))
                    .map(|(_, regime)| regime)
                    .collect();
                let Some(reasoning) = self.reasoning.as_ref().filter(|_| !listed.is_empty()) else {
                    return Ok(Outcome::Unsupported(format!(
                        "entailment regime {} needs a reasoner for RDFS or OWL-RDF-Based",
                        regimes.join(" ")
                    )));
                };
                let Some(regime) = listed.iter().copied().find(|r| reasoning.supports(*r)) else {
                    return Ok(Outcome::Unsupported(format!(
                        "reasoner offers no {:?} profile",
                        listed[0]
                    )));
                };
                // Materialise the default graph's closure, then answer the query over it.
                let default = data
                    .store()
                    .quads_for_pattern(None, None, None, Some(GraphNameRef::DefaultGraph))
                    .collect::<Result<Vec<_>, _>>()?;
                match reasoning.derive(regime, &entailment::ntriples(default)?)? {
                    Derivation::Derived(derived) => {
                        for quad in entailment::parse(&derived, RdfFormat::NTriples, "urn:x:")? {
                            data.store().insert(&quad)?;
                        }
                    }
                    Derivation::Rejected(why) => {
                        return Ok(Outcome::Unsupported(format!(
                            "outside the reasoner profile: {why}"
                        )));
                    }
                    Derivation::Inconsistent(why) => {
                        bail!("reasoner reported the data inconsistent: {why}")
                    }
                }
            }
            let query_path = self.local(&one(manifest, &action, &format!("{QT}query"))?)?;
            let query = self.text(&query_path)?;
            for graph in dataset_clauses(&query)? {
                let Ok(file) = self.local(&Term::NamedNode(graph.clone())) else { continue };
                if file.is_file() && !data.store().contains_named_graph(&graph)? {
                    self.load(&data, &file, GraphName::NamedNode(graph))?;
                }
            }
            let expected = self.local(&one(manifest, entry, &format!("{MF}result"))?)?;
            return match self.engine {
                Engine::Oxigraph => {
                    let results = SparqlEvaluator::new()
                        .parse_query(&query)?
                        .on_store(data.store())
                        .execute()?;
                    self.compare(results, &query, &expected)
                }
                Engine::Database => {
                    let database = database(data)?;
                    let results = database.query(&query)?;
                    self.compare(results, &query, &expected)
                }
            };
        }
        if kind.contains("UpdateEvaluationTest") {
            let data = self.dataset_description(manifest, &action, UT)?;
            let update =
                self.text(&self.local(&one(manifest, &action, &format!("{UT}request"))?)?)?;
            let store = data.store().clone();
            match self.engine {
                Engine::Oxigraph => {
                    SparqlEvaluator::new().parse_update(&update)?.on_store(&store).execute()?
                }
                Engine::Database => database(data)?.update(&update)?,
            }
            let expected = self.dataset_description(
                manifest,
                &one(manifest, entry, &format!("{MF}result"))?,
                UT,
            )?;
            let (actual, expected) = (quads(&store)?, quads(expected.store())?);
            ensure!(actual == expected, "update result mismatch: {}", diff(&expected, &actual));
            return Ok(Outcome::Passed);
        }
        if kind.contains("Eval") && matches!(action, Term::NamedNode(_)) {
            let actual = Dataset::memory()?;
            let expected = Dataset::memory()?;
            self.load(&actual, &self.local(&action)?, GraphName::DefaultGraph)?;
            self.load(
                &expected,
                &self.local(&one(manifest, entry, &format!("{MF}result"))?)?,
                GraphName::DefaultGraph,
            )?;
            ensure!(quads(actual.store())? == quads(expected.store())?, "RDF evaluation mismatch");
            return Ok(Outcome::Passed);
        }
        if kind == format!("<{MF}PositiveEntailmentTest>")
            || kind == format!("<{MF}NegativeEntailmentTest>")
        {
            return self.rdf_entailment(manifest, entry, &action, kind.contains("Positive"));
        }
        Ok(Outcome::Unsupported(format!("unsupported manifest case type: {kind}")))
    }
    /// RDF 1.1 Semantics tests: does the premise entail the conclusion under the regime?
    fn rdf_entailment(
        &self,
        manifest: &Dataset,
        entry: &Term,
        action: &Term,
        positive: bool,
    ) -> Result<Outcome> {
        let regime = match one(manifest, entry, &format!("{MF}entailmentRegime"))? {
            Term::Literal(l) => l.value().to_owned(),
            other => other.to_string(),
        };
        let Some(reasoning) = &self.reasoning else {
            return Ok(Outcome::Unsupported(format!("{regime} entailment needs a reasoner")));
        };
        if regime != "RDFS" {
            return Ok(Outcome::Unsupported(format!(
                "{regime} entailment: no matching profile (only RDFS is offered; using it would over-entail)"
            )));
        }
        if !reasoning.supports(Regime::Rdfs) {
            return Ok(Outcome::Unsupported("reasoner offers no Rdfs profile".into()));
        }
        let datatypes = match one(manifest, entry, &format!("{MF}recognizedDatatypes")) {
            Ok(list) => terms(manifest, list)?,
            Err(_) => Vec::new(),
        }
        .iter()
        .map(Term::to_string)
        .filter(|d| !d.ends_with("#string>") && !d.ends_with("#langString>"))
        .collect::<Vec<_>>();
        let premise_path = self.local(action)?;
        let premise = entailment::parse(
            &fs::read_to_string(&premise_path)?,
            format(&premise_path)?,
            &self.iri(&premise_path)?,
        )?;
        let context = if datatypes.is_empty() {
            String::new()
        } else {
            format!(" (recognized datatypes {})", datatypes.join(" "))
        };
        let derivation = reasoning.derive(Regime::Rdfs, &entailment::ntriples(premise.clone())?)?;
        let result = one(manifest, entry, &format!("{MF}result"))?;
        if let Term::Literal(_) = result {
            // `false`: the premise is unsatisfiable.
            return match (derivation, positive) {
                (Derivation::Inconsistent(_), true) | (Derivation::Derived(_), false) => {
                    Ok(Outcome::Passed)
                }
                (Derivation::Rejected(why), _) => {
                    Ok(Outcome::Unsupported(format!("outside the reasoner profile: {why}")))
                }
                (Derivation::Derived(_), true) => bail!("inconsistency not detected{context}"),
                (Derivation::Inconsistent(why), false) => {
                    bail!("reported inconsistent, wrongly: {why}{context}")
                }
            };
        }
        let conclusion_path = self.local(&result)?;
        let conclusion = entailment::parse(
            &fs::read_to_string(&conclusion_path)?,
            format(&conclusion_path)?,
            &self.iri(&conclusion_path)?,
        )?;
        let derived = match derivation {
            Derivation::Derived(derived) => derived,
            Derivation::Rejected(why) => {
                return Ok(Outcome::Unsupported(format!("outside the reasoner profile: {why}")));
            }
            // An inconsistent premise entails everything.
            Derivation::Inconsistent(_) if positive => return Ok(Outcome::Passed),
            Derivation::Inconsistent(why) => {
                bail!("reported inconsistent, wrongly: {why}{context}")
            }
        };
        let mut closure = premise;
        closure.extend(entailment::parse(&derived, RdfFormat::NTriples, "urn:x:")?);
        let entailed = entailment::entails(&closure, &conclusion)?;
        let unmatched = entailed
            .as_ref()
            .err()
            .map(|u| format!("; unmatched: {}", u.join(" | ")))
            .unwrap_or_default();
        let entailed = entailed.is_ok();
        ensure!(
            entailed == positive,
            "{}{context}{unmatched}",
            if positive {
                "conclusion not entailed"
            } else {
                "unsound: a non-entailed conclusion was derived"
            }
        );
        Ok(Outcome::Passed)
    }
    /// OWL 2 conformance export (`test:TestCase` resources with embedded RDF/XML ontologies).
    fn owl2(&self, cases: &Dataset, report: &mut Report) -> Result<()> {
        let test = |name: &str| format!("{OWLTEST}{name}");
        let kinds = [
            "PositiveEntailmentTest",
            "NegativeEntailmentTest",
            "ConsistencyTest",
            "InconsistencyTest",
        ];
        let subjects = cases
            .store()
            .quads_for_pattern(
                None,
                Some(NamedNode::new(format!("{RDF}type"))?.as_ref()),
                Some(NamedNode::new(test("TestCase"))?.as_ref().into()),
                None,
            )
            .map(|q| q.map(|q| Term::from(q.subject)))
            .collect::<Result<Vec<_>, _>>()?;
        for subject in subjects {
            let types = objects(cases, &subject, &format!("{RDF}type"))?
                .iter()
                .map(Term::to_string)
                .collect::<Vec<_>>();
            for kind in kinds.iter().filter(|k| types.contains(&format!("<{}>", test(k)))) {
                let outcome = self.owl2_case(cases, &subject, kind);
                record(
                    report,
                    format!("{}#{kind}", subject.to_string().trim_end_matches('>')) + ">",
                    outcome,
                );
            }
        }
        Ok(())
    }
    fn owl2_case(&self, cases: &Dataset, subject: &Term, kind: &str) -> Result<Outcome> {
        let test = |name: &str| format!("{OWLTEST}{name}");
        let Some(reasoning) = &self.reasoning else {
            return Ok(Outcome::Unsupported("OWL entailment needs a reasoner".into()));
        };
        if !reasoning.supports(Regime::OwlRl) {
            return Ok(Outcome::Unsupported("reasoner offers no OwlRl profile".into()));
        }
        let semantics = objects(cases, subject, &test("semantics"))?
            .iter()
            .map(Term::to_string)
            .collect::<Vec<_>>();
        if !semantics.contains(&format!("<{}>", test("RDF-BASED"))) {
            return Ok(Outcome::Unsupported(
                "Direct Semantics only: OWL 2 RL rules implement the RDF-Based Semantics".into(),
            ));
        }
        if !objects(cases, subject, &test("importedOntology"))?.is_empty() {
            return Ok(Outcome::Unsupported(
                "uses owl:imports; the reasoner fetches nothing".into(),
            ));
        }
        let ontology = |property: &str| -> Result<Option<Vec<Quad>>> {
            match objects(cases, subject, &test(property))?.into_iter().next() {
                Some(Term::Literal(text)) => {
                    let quads = entailment::parse(
                        text.value(),
                        RdfFormat::RdfXml,
                        "http://owl.semanticweb.org/id/",
                    )?;
                    // Ontology headers are not claims about the domain.
                    let header = NamedNode::new(format!("{OWL}Ontology"))?;
                    Ok(Some(
                        quads
                            .into_iter()
                            .filter(|q| q.object != Term::from(header.clone()))
                            .collect(),
                    ))
                }
                _ => Ok(None),
            }
        };
        let Some(premise) = ontology("rdfXmlPremiseOntology")? else {
            return Ok(Outcome::Unsupported("premise given in functional syntax only".into()));
        };
        let derivation =
            reasoning.derive(Regime::OwlRl, &entailment::ntriples(premise.clone())?)?;
        match (kind, derivation) {
            (_, Derivation::Rejected(why)) => {
                Ok(Outcome::Unsupported(format!("outside the reasoner profile: {why}")))
            }
            ("ConsistencyTest", Derivation::Inconsistent(why)) => {
                bail!("reported inconsistent, wrongly: {why}")
            }
            ("InconsistencyTest", Derivation::Inconsistent(_)) | ("ConsistencyTest", _) => {
                Ok(Outcome::Passed)
            }
            ("InconsistencyTest", _) => bail!("inconsistency not detected"),
            ("PositiveEntailmentTest", Derivation::Inconsistent(_)) => Ok(Outcome::Passed),
            ("NegativeEntailmentTest", Derivation::Inconsistent(why)) => {
                bail!("reported inconsistent, wrongly: {why}")
            }
            (_, Derivation::Derived(derived)) => {
                let positive = kind == "PositiveEntailmentTest";
                let property = if positive {
                    "rdfXmlConclusionOntology"
                } else {
                    "rdfXmlNonConclusionOntology"
                };
                let Some(conclusion) = ontology(property)? else {
                    return Ok(Outcome::Unsupported(
                        "conclusion given in functional syntax only".into(),
                    ));
                };
                let mut closure = premise;
                closure.extend(entailment::parse(&derived, RdfFormat::NTriples, "urn:x:")?);
                let entailed = entailment::entails(&closure, &conclusion)?;
                let unmatched = entailed
                    .as_ref()
                    .err()
                    .map(|u| format!("; unmatched: {}", u.join(" | ")))
                    .unwrap_or_default();
                let entailed = entailed.is_ok();
                ensure!(
                    entailed == positive,
                    "{}{unmatched}",
                    if positive {
                        "conclusion not entailed"
                    } else {
                        "unsound: the non-conclusion was entailed"
                    }
                );
                Ok(Outcome::Passed)
            }
            (other, Derivation::Inconsistent(_)) => bail!("unknown OWL test kind {other}"),
        }
    }
    fn compare(&self, results: QueryResults<'_>, query: &str, expected: &Path) -> Result<Outcome> {
        use comparison::Literals::{ByValue, Exact};
        let extension = expected.extension().and_then(|x| x.to_str()).unwrap_or_default();
        if extension == "csv" {
            let wanted = fs::read_to_string(expected)?;
            let header = cells(wanted.lines().next().unwrap_or_default());
            let (actual, wanted) = (csv(results, &header)?, csv_text(&wanted, &header));
            ensure!(actual == wanted, "query result mismatch: {}", diff(&wanted, &actual));
            return Ok(Outcome::Passed);
        }
        // Results are consumed once: materialise them, then canonicalise both ways.
        let actual = Materialised::from(results)?;
        let wanted = |literals| -> Result<String> {
            match extension {
                "srx" | "srj" | "tsv" => {
                    let mime = match extension {
                        "srx" => "application/sparql-results+xml",
                        "srj" => "application/sparql-results+json",
                        _ => "text/tab-separated-values",
                    };
                    comparison::from_bytes_as(&fs::read(expected)?, mime, query, literals)
                }
                _ if matches!(actual, Materialised::Graph(_)) => {
                    let graph = Dataset::memory()?;
                    self.load(&graph, expected, GraphName::DefaultGraph)?;
                    comparison::graph(
                        graph.store().iter().collect::<Result<Vec<_>, _>>()?,
                        literals,
                    )
                }
                _ => self.result_set(expected, query, literals),
            }
        };
        let (exact, wanted_exact) = (actual.canonical(query, Exact)?, wanted(Exact)?);
        if exact == wanted_exact {
            return Ok(Outcome::Passed);
        }
        let (by_value, wanted_by_value) = (actual.canonical(query, ByValue)?, wanted(ByValue)?);
        ensure!(
            by_value == wanted_by_value,
            "query result mismatch: {}",
            diff(&wanted_exact, &exact)
        );
        Ok(Outcome::ValueOnly(format!("lexical forms differ: {}", diff(&wanted_exact, &exact))))
    }
    /// A result set written in RDF with the DAWG `rs:` vocabulary.
    fn result_set(
        &self,
        path: &Path,
        query: &str,
        literals: comparison::Literals,
    ) -> Result<String> {
        let graph = Dataset::memory()?;
        self.load(&graph, path, GraphName::DefaultGraph)?;
        let set = graph
            .store()
            .quads_for_pattern(
                None,
                Some(NamedNode::new(format!("{RDF}type"))?.as_ref()),
                Some(NamedNode::new(format!("{RS}ResultSet"))?.as_ref().into()),
                None,
            )
            .next()
            .context("expected result has no rs:ResultSet")??
            .subject;
        let set: Term = set.into();
        if let Ok(Term::Literal(value)) = one(&graph, &set, &format!("{RS}boolean")) {
            return Ok(format!("boolean:{}", value.value()));
        }
        let literal = |t: Term| match t {
            Term::Literal(l) => Ok(l.value().to_owned()),
            other => bail!("expected a literal, found {other}"),
        };
        let variables = objects(&graph, &set, &format!("{RS}resultVariable"))?
            .into_iter()
            .map(literal)
            .collect::<Result<Vec<_>>>()?;
        let mut rows = Vec::new();
        for solution in objects(&graph, &set, &format!("{RS}solution"))? {
            let index = match one(&graph, &solution, &format!("{RS}index")) {
                Ok(Term::Literal(l)) => l.value().parse::<usize>().unwrap_or(usize::MAX),
                _ => usize::MAX,
            };
            let mut row = Vec::new();
            for binding in objects(&graph, &solution, &format!("{RS}binding"))? {
                row.push((
                    literal(one(&graph, &binding, &format!("{RS}variable"))?)?,
                    one(&graph, &binding, &format!("{RS}value"))?,
                ));
            }
            rows.push((index, row));
        }
        rows.sort_by_key(|(index, _)| *index);
        comparison::solutions_for(
            variables,
            rows.into_iter().map(|(_, row)| row).collect(),
            query,
            literals,
        )
    }
}
/// Query results held in memory so they can be canonicalised more than once.
enum Materialised {
    Boolean(bool),
    Graph(Vec<Quad>),
    Solutions(Vec<String>, Vec<Vec<(String, Term)>>),
}
impl Materialised {
    fn from(results: QueryResults<'_>) -> Result<Self> {
        Ok(match results {
            QueryResults::Boolean(value) => Self::Boolean(value),
            QueryResults::Graph(triples) => Self::Graph(
                triples
                    .map(|t| {
                        t.map(|t| {
                            Quad::new(t.subject, t.predicate, t.object, GraphName::DefaultGraph)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            QueryResults::Solutions(solutions) => {
                let variables =
                    solutions.variables().iter().map(|v| v.as_str().to_owned()).collect();
                let rows = solutions
                    .map(|row| {
                        row.map(|row| {
                            row.iter().map(|(v, t)| (v.as_str().to_owned(), t.clone())).collect()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Self::Solutions(variables, rows)
            }
        })
    }
    fn canonical(&self, query: &str, literals: comparison::Literals) -> Result<String> {
        match self {
            Self::Boolean(value) => Ok(format!("boolean:{value}")),
            Self::Graph(quads) => comparison::graph(quads.clone(), literals),
            Self::Solutions(variables, rows) => {
                comparison::solutions_for(variables.clone(), rows.clone(), query, literals)
            }
        }
    }
}
/// CSV is lossy (no datatypes, no term kinds), so compare text: columns matched by header name
/// (`SELECT *` leaves column order open), rows as a multiset, blank-node labels erased.
fn csv(results: QueryResults<'_>, header: &[String]) -> Result<String> {
    let QueryResults::Solutions(solutions) = results else { bail!("CSV results need a SELECT") };
    let mut serializer = QueryResultsSerializer::from_format(QueryResultsFormat::Csv)
        .serialize_solutions_to_writer(Vec::new(), solutions.variables().to_vec())?;
    for solution in solutions {
        serializer.serialize(&solution?)?;
    }
    Ok(csv_text(&String::from_utf8(serializer.finish()?)?, header))
}
fn csv_text(text: &str, header: &[String]) -> String {
    let mut lines = text.lines().filter(|l| !l.is_empty());
    let own = cells(lines.next().unwrap_or_default());
    let order = header.iter().map(|h| own.iter().position(|o| o == h)).collect::<Vec<_>>();
    let mut rows = lines
        .map(|line| {
            let row = cells(line);
            order
                .iter()
                .map(|i| {
                    let cell =
                        i.and_then(|i| row.get(i)).map(String::as_str).unwrap_or("<missing>");
                    if cell.starts_with("_:") { "_:b" } else { cell }
                })
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
        .collect::<Vec<_>>();
    rows.sort();
    format!("{}\n{}", header.join(","), rows.join("\n"))
}
/// RFC 4180 cells: commas separate, quotes enclose, doubled quotes escape.
fn cells(line: &str) -> Vec<String> {
    let (mut cells, mut cell, mut quoted, mut chars) =
        (Vec::new(), String::new(), false, line.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => cells.push(std::mem::take(&mut cell)),
            _ => cell.push(c),
        }
    }
    cells.push(cell);
    cells
}
