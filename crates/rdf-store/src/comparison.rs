//! Result comparison preserving multiplicity, variable scope and blank-node identity.
use crate::engine::canonical_quads;
use anyhow::Result;
use oxigraph::{
    io::{RdfFormat, RdfParser},
    model::*,
    sparql::{
        QueryResults,
        results::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput},
    },
};

/// True when the outermost modifier is REDUCED: duplicates may or may not be removed, so the
/// harness compares solutions as sets.
fn reduced(query: &str) -> Result<bool> {
    use spargebra::{Query, algebra::GraphPattern};
    fn outer(pattern: &GraphPattern) -> bool {
        match pattern {
            GraphPattern::Reduced { .. } => true,
            GraphPattern::Project { inner, .. } | GraphPattern::Slice { inner, .. } => outer(inner),
            _ => false,
        }
    }
    Ok(match spargebra::SparqlParser::new().parse_query(query)? {
        Query::Select { pattern, .. } => outer(&pattern),
        _ => false,
    })
}
fn deduplicated(rows: Vec<Vec<(String, Term)>>, query: &str) -> Result<Vec<Vec<(String, Term)>>> {
    if !reduced(query)? {
        return Ok(rows);
    }
    let mut seen = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|row| {
            let mut key = row.iter().map(|(v, t)| format!("{v}={t}")).collect::<Vec<_>>();
            key.sort();
            seen.insert(key)
        })
        .collect())
}
fn ordered(query: &str) -> Result<bool> {
    use spargebra::{Query, algebra::GraphPattern};
    fn outer(pattern: &GraphPattern) -> bool {
        match pattern {
            GraphPattern::OrderBy { .. } => true,
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => outer(inner),
            _ => false,
        }
    }
    Ok(match spargebra::SparqlParser::new().parse_query(query)? {
        Query::Select { pattern, .. } => outer(&pattern),
        _ => false,
    })
}
/// How literals are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Literals {
    /// Term equality: lexical forms must match.
    Exact,
    /// Numeric and boolean literals compare by value (`"1.0"^^xsd:decimal` = `"1"^^xsd:decimal`),
    /// as the W3C SPARQL results-comparison guidance allows. Ill-formed values stay as written.
    ByValue,
}
/// Canonical form of a solution sequence for `query`: row order counts under an outer ORDER BY,
/// duplicates do not under REDUCED.
pub fn solutions_for(
    variables: Vec<String>,
    rows: Vec<Vec<(String, Term)>>,
    query: &str,
    literals: Literals,
) -> Result<String> {
    self::rows(variables, normalized(deduplicated(rows, query)?, literals), ordered(query)?)
}
fn normalized(rows: Vec<Vec<(String, Term)>>, literals: Literals) -> Vec<Vec<(String, Term)>> {
    if literals == Literals::Exact {
        return rows;
    }
    rows.into_iter().map(|row| row.into_iter().map(|(v, t)| (v, by_value(t))).collect()).collect()
}
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const INTEGERS: [&str; 13] = [
    "integer",
    "int",
    "long",
    "short",
    "byte",
    "nonNegativeInteger",
    "positiveInteger",
    "nonPositiveInteger",
    "negativeInteger",
    "unsignedLong",
    "unsignedInt",
    "unsignedShort",
    "unsignedByte",
];
/// One lexical form per value for numeric and boolean literals.
pub fn by_value(term: Term) -> Term {
    let Term::Literal(literal) = &term else { return term };
    let Some(kind) = literal.datatype().as_str().strip_prefix(XSD) else { return term };
    let lexical = literal.value().trim();
    let canonical = if INTEGERS.contains(&kind) {
        lexical.parse::<i128>().ok().map(|v| v.to_string())
    } else if kind == "decimal" {
        decimal(lexical)
    } else if kind == "double" {
        lexical.parse::<f64>().ok().map(|v| format!("{v:e}"))
    } else if kind == "float" {
        lexical.parse::<f32>().ok().map(|v| format!("{v:e}"))
    } else if kind == "boolean" {
        match lexical {
            "true" | "1" => Some("true".to_owned()),
            "false" | "0" => Some("false".to_owned()),
            _ => None,
        }
    } else {
        None
    };
    match canonical {
        Some(value) => Literal::new_typed_literal(value, literal.datatype().into_owned()).into(),
        None => term,
    }
}
fn decimal(lexical: &str) -> Option<String> {
    let (negative, digits) = match lexical.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, lexical.strip_prefix('+').unwrap_or(lexical)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.chars().chain(fraction.chars()).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let zero = whole == "0" && fraction.is_empty();
    let sign = if negative && !zero { "-" } else { "" };
    Some(if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    })
}
fn rows(variables: Vec<String>, rows: Vec<Vec<(String, Term)>>, ordered: bool) -> Result<String> {
    let mut variables = variables;
    variables.sort();
    let mut graph = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let subject = BlankNode::default();
        graph.push(Quad::new(
            subject.clone(),
            NamedNode::new("urn:result:row")?,
            Literal::new_simple_literal(if ordered { index.to_string() } else { String::new() }),
            GraphName::DefaultGraph,
        ));
        for (variable, value) in row {
            graph.push(Quad::new(
                subject.clone(),
                NamedNode::new(format!("urn:binding:{variable}"))?,
                value,
                GraphName::DefaultGraph,
            ));
        }
    }
    Ok(format!("solutions:{variables:?}\n{}", canonical_quads(graph)))
}
pub fn from_results(results: QueryResults<'_>, query: &str) -> Result<String> {
    from_results_as(results, query, Literals::Exact)
}
pub fn from_results_as(
    results: QueryResults<'_>,
    query: &str,
    literals: Literals,
) -> Result<String> {
    match results {
        QueryResults::Boolean(value) => Ok(format!("boolean:{value}")),
        QueryResults::Graph(triples) => graph(
            triples
                .map(|t| {
                    t.map(|t| Quad::new(t.subject, t.predicate, t.object, GraphName::DefaultGraph))
                })
                .collect::<Result<Vec<_>, _>>()?,
            literals,
        ),
        QueryResults::Solutions(solutions) => {
            let variables = solutions.variables().iter().map(|v| v.as_str().to_owned()).collect();
            let values = solutions
                .map(|row| {
                    row.map(|row| {
                        row.iter().map(|(v, t)| (v.as_str().to_owned(), t.clone())).collect()
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            solutions_for(variables, values, query, literals)
        }
    }
}
pub fn from_bytes(bytes: &[u8], mime: &str, query: &str) -> Result<String> {
    from_bytes_as(bytes, mime, query, Literals::Exact)
}
pub fn from_bytes_as(bytes: &[u8], mime: &str, query: &str, literals: Literals) -> Result<String> {
    if let Some(format) = QueryResultsFormat::from_media_type(mime) {
        match QueryResultsParser::from_format(format).for_reader(bytes)? {
            ReaderQueryResultsParserOutput::Boolean(value) => Ok(format!("boolean:{value}")),
            ReaderQueryResultsParserOutput::Solutions(solutions) => {
                let variables =
                    solutions.variables().iter().map(|v| v.as_str().to_owned()).collect();
                let values = solutions
                    .map(|row| {
                        row.map(|row| {
                            row.iter().map(|(v, t)| (v.as_str().to_owned(), t.clone())).collect()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                solutions_for(variables, values, query, literals)
            }
        }
    } else {
        let format = RdfFormat::from_media_type(mime)
            .ok_or_else(|| anyhow::anyhow!("unknown result MIME type: {mime}"))?;
        graph(
            RdfParser::from_format(format).for_reader(bytes).collect::<Result<Vec<_>, _>>()?,
            literals,
        )
    }
}
/// Canonical form of an RDF graph or dataset result.
pub fn graph(quads: Vec<Quad>, literals: Literals) -> Result<String> {
    let quads = quads.into_iter().map(|q| match literals {
        Literals::Exact => q,
        Literals::ByValue => Quad::new(q.subject, q.predicate, by_value(q.object), q.graph_name),
    });
    Ok(format!("graph:{}", canonical_quads(quads)))
}
