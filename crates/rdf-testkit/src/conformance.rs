//! RDF manifest interpretation. Unsupported cases are never counted as passes.
use anyhow::{Context, Result, bail, ensure};
use rdf_store::{
    comparison,
    engine::{Dataset, canonical_quads, format_for_path},
    oxigraph::{
        io::RdfParser,
        model::*,
        sparql::{QueryResults, SparqlEvaluator},
    },
};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const QT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-query#";
const UT: &str = "http://www.w3.org/2009/sparql/tests/test-update#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
#[derive(Serialize, Default)]
pub struct Report {
    pub passed: usize,
    pub failed: usize,
    pub unsupported: usize,
    pub skipped: usize,
    pub cases: Vec<Case>,
}
#[derive(Serialize)]
pub struct Case {
    pub id: String,
    pub status: String,
    pub detail: String,
}
pub fn run(path: &Path) -> Result<Report> {
    let mut report = Report::default();
    visit(path, &mut HashSet::new(), &mut report)?;
    Ok(report)
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
    objects(data, subject, predicate)?.into_iter().next().context("missing manifest property")
}
fn local(term: &Term) -> Result<PathBuf> {
    let Term::NamedNode(node) = term else { bail!("expected file IRI") };
    url::Url::parse(node.as_str())?
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("only local fixture URLs are allowed: {node}"))
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
fn load(data: &Dataset, path: &Path, graph: GraphName) -> Result<()> {
    let canonical = path.canonicalize()?;
    let mut iri =
        url::Url::from_file_path(&canonical).map_err(|_| anyhow::anyhow!("invalid path"))?;
    // Manifest links stay local; fixture semantics use the suite's published base.
    if path.file_name().is_none_or(|name| name != "manifest.ttl")
        && let Some(root) = canonical.ancestors().find(|p| p.join("test-map.json").is_file())
    {
        iri = url::Url::parse("https://w3c.github.io/rdf-tests/")?
            .join(&canonical.strip_prefix(root)?.to_string_lossy())?;
    }
    let parser = RdfParser::from_format(format_for_path(path)?)
        .with_base_iri(iri.as_str())?
        .with_default_graph(graph);
    data.store().load_from_reader(parser, fs::File::open(path)?)?;
    Ok(())
}
fn visit(path: &Path, seen: &mut HashSet<PathBuf>, report: &mut Report) -> Result<()> {
    let path = path.canonicalize()?;
    if !seen.insert(path.clone()) {
        return Ok(());
    }
    let manifest = Dataset::memory()?;
    load(&manifest, &path, GraphName::DefaultGraph)?;
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
    ensure!(!entries.is_empty() || !includes.is_empty(), "manifest has no entries or includes");
    for entry in entries {
        let kind = one(&manifest, &entry, &format!("{RDF}type"))?.to_string();
        let result = case(&manifest, &entry, &kind);
        let (status, detail) = match result {
            Ok(Some(())) => {
                report.passed += 1;
                ("passed", String::new())
            }
            Ok(None) => {
                report.unsupported += 1;
                ("unsupported", format!("unsupported manifest case type: {kind}"))
            }
            Err(error) => {
                report.failed += 1;
                ("failed", format!("{error:#}"))
            }
        };
        report.cases.push(Case { id: entry.to_string(), status: status.into(), detail });
    }
    for include in includes {
        visit(&local(&include)?, seen, report)?;
    }
    Ok(())
}
fn dataset_description(manifest: &Dataset, node: &Term, ns: &str) -> Result<Dataset> {
    let data = Dataset::memory()?;
    for file in objects(manifest, node, &format!("{ns}data"))? {
        load(&data, &local(&file)?, GraphName::DefaultGraph)?;
    }
    for graph_data in objects(manifest, node, &format!("{ns}graphData"))? {
        if matches!(graph_data, Term::NamedNode(_)) && ns == QT {
            let Term::NamedNode(name) = &graph_data else { unreachable!() };
            load(&data, &local(&graph_data)?, GraphName::NamedNode(name.clone()))?;
        } else {
            let file = one(manifest, &graph_data, &format!("{ns}graph"))?;
            let name = one(manifest, &graph_data, "http://www.w3.org/2000/01/rdf-schema#label")?;
            let name = match name {
                Term::NamedNode(n) => n,
                Term::Literal(l) => NamedNode::new(l.value())?,
                _ => bail!("invalid graph name"),
            };
            data.store().insert_named_graph(&name)?;
            load(&data, &local(&file)?, GraphName::NamedNode(name))?;
        }
    }
    Ok(data)
}
fn case(manifest: &Dataset, entry: &Term, kind: &str) -> Result<Option<()>> {
    let action = one(manifest, entry, &format!("{MF}action"))?;
    if kind.contains("SyntaxTest")
        || kind.contains("PositiveSyntax")
        || kind.contains("NegativeSyntax")
        || kind.contains("NegativeEval")
    {
        let file = local(&action)?;
        let input = fs::read_to_string(&file)?;
        let negative = kind.contains("Negative") || kind.contains("NegativeEval");
        let valid = if file.extension().is_some_and(|e| e == "rq") {
            SparqlEvaluator::new().with_base_iri("http://example.org/")?.parse_query(&input).is_ok()
        } else if file.extension().is_some_and(|e| e == "ru") {
            SparqlEvaluator::new()
                .with_base_iri("http://example.org/")?
                .parse_update(&input)
                .is_ok()
        } else {
            let iri = url::Url::from_file_path(&file)
                .map_err(|_| anyhow::anyhow!("invalid fixture path"))?;
            RdfParser::from_format(format_for_path(&file)?)
                .with_base_iri(iri.as_str())?
                .for_reader(input.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .is_ok()
        };
        ensure!(valid != negative, "syntax expectation mismatch");
        return Ok(Some(()));
    }
    if kind.contains("QueryEvaluationTest") {
        let query_path = local(&one(manifest, &action, &format!("{QT}query"))?)?;
        let query = fs::read_to_string(&query_path)?;
        let data = dataset_description(manifest, &action, QT)?;
        let base = url::Url::from_file_path(&query_path)
            .map_err(|_| anyhow::anyhow!("invalid query path"))?;
        let result = SparqlEvaluator::new()
            .with_base_iri(base.as_str())?
            .parse_query(&query)?
            .on_store(data.store())
            .execute()?;
        let expected = local(&one(manifest, entry, &format!("{MF}result"))?)?;
        let mime = match expected.extension().and_then(|x| x.to_str()) {
            Some("srx") => "application/sparql-results+xml",
            Some("srj") => "application/sparql-results+json",
            Some("ttl") if matches!(&result, QueryResults::Graph(_)) => "text/turtle",
            Some("rdf") => "application/rdf+xml",
            _ => return Ok(None),
        };
        ensure!(
            comparison::from_results(result, &query)?
                == comparison::from_bytes(&fs::read(expected)?, mime, &query)?,
            "query result mismatch"
        );
        return Ok(Some(()));
    }
    if kind.contains("UpdateEvaluationTest") {
        let data = dataset_description(manifest, &action, UT)?;
        let request = local(&one(manifest, &action, &format!("{UT}request"))?)?;
        data.update(&fs::read_to_string(request)?)?;
        let expected =
            dataset_description(manifest, &one(manifest, entry, &format!("{MF}result"))?, UT)?;
        ensure!(
            canonical_quads(data.store().iter().collect::<Result<Vec<_>, _>>()?)
                == canonical_quads(expected.store().iter().collect::<Result<Vec<_>, _>>()?),
            "update result mismatch"
        );
        return Ok(Some(()));
    }
    if kind.contains("Eval") && matches!(action, Term::NamedNode(_)) {
        let actual = Dataset::memory()?;
        let expected = Dataset::memory()?;
        load(&actual, &local(&action)?, GraphName::DefaultGraph)?;
        load(
            &expected,
            &local(&one(manifest, entry, &format!("{MF}result"))?)?,
            GraphName::DefaultGraph,
        )?;
        ensure!(
            canonical_quads(actual.store().iter().collect::<Result<Vec<_>, _>>()?)
                == canonical_quads(expected.store().iter().collect::<Result<Vec<_>, _>>()?),
            "RDF evaluation mismatch"
        );
        return Ok(Some(()));
    }
    Ok(None)
}
