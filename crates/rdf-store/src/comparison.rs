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
    match results {
        QueryResults::Boolean(value) => Ok(format!("boolean:{value}")),
        QueryResults::Graph(triples) => Ok(format!(
            "graph:{}",
            canonical_quads(
                triples
                    .map(|t| t.map(|t| Quad::new(
                        t.subject,
                        t.predicate,
                        t.object,
                        GraphName::DefaultGraph
                    )))
                    .collect::<Result<Vec<_>, _>>()?
            )
        )),
        QueryResults::Solutions(solutions) => {
            let variables = solutions.variables().iter().map(|v| v.as_str().to_owned()).collect();
            let values = solutions
                .map(|row| {
                    row.map(|row| {
                        row.iter().map(|(v, t)| (v.as_str().to_owned(), t.clone())).collect()
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            rows(variables, values, ordered(query)?)
        }
    }
}
pub fn from_bytes(bytes: &[u8], mime: &str, query: &str) -> Result<String> {
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
                rows(variables, values, ordered(query)?)
            }
        }
    } else {
        let format = RdfFormat::from_media_type(mime)
            .ok_or_else(|| anyhow::anyhow!("unknown result MIME type: {mime}"))?;
        Ok(format!(
            "graph:{}",
            canonical_quads(
                RdfParser::from_format(format).for_reader(bytes).collect::<Result<Vec<_>, _>>()?
            )
        ))
    }
}
