//! Reading an agent's SPARQL with the engine's own parser: what kind of query it is, where it must
//! be sent, which terms it mentions, and how to give it a dataset. Nothing here talks to a server.
use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
    QueryDataset,
};
use spargebra::term::{GroundTerm, NamedNode, NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};
use std::collections::{BTreeMap, BTreeSet};

pub const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
pub const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
pub const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
/// Query text larger than this is refused before parsing.
pub const MAX_QUERY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Select,
    Construct,
    Describe,
    Ask,
}
impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Form::Select => "SELECT",
            Form::Construct => "CONSTRUCT",
            Form::Describe => "DESCRIBE",
            Form::Ask => "ASK",
        }
    }
}

/// A call site of a property function: `(?out ...) <iri> (in ...)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCall {
    pub iri: String,
    /// Number of items in the input list, when it is a list.
    pub inputs: Option<usize>,
    /// Number of items in the output list, when it is a list.
    pub outputs: Option<usize>,
    /// Inputs that are variables (or the whole input is a variable); the engine needs constants.
    pub variable_inputs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Endpoint IRIs named by `SERVICE`.
    pub services: BTreeSet<String>,
    /// `SERVICE ?var` was used; the endpoint is unknown until run time.
    pub service_variable: bool,
    /// The query brings its own `FROM` / `FROM NAMED`.
    pub has_dataset: bool,
    /// Graph IRIs named by `GRAPH <g>` or `FROM`.
    pub graphs: BTreeSet<String>,
    /// IRIs used as subjects, predicates, objects, path steps or values. Terms inside a `SERVICE`
    /// block are in `remote_terms` instead, because they belong to another endpoint.
    pub terms: BTreeSet<String>,
    pub remote_terms: BTreeSet<String>,
    pub functions: Vec<FunctionCall>,
}

/// Parse a query. Updates are recognised so the refusal can say why.
pub fn parse(text: &str) -> Result<(Query, Form), String> {
    if text.len() > MAX_QUERY_BYTES {
        return Err(format!("query exceeds {} KiB", MAX_QUERY_BYTES / 1024));
    }
    match SparqlParser::new().parse_query(text) {
        Ok(query) => {
            let form = match &query {
                Query::Select { .. } => Form::Select,
                Query::Construct { .. } => Form::Construct,
                Query::Describe { .. } => Form::Describe,
                Query::Ask { .. } => Form::Ask,
            };
            Ok((query, form))
        }
        Err(error) => {
            if SparqlParser::new().parse_update(text).is_ok() {
                Err("this is a SPARQL update; this server is read-only and accepts only SELECT, ASK, CONSTRUCT and DESCRIBE".into())
            } else {
                Err(error.to_string())
            }
        }
    }
}

fn parts(query: &Query) -> (&Option<QueryDataset>, &GraphPattern) {
    match query {
        Query::Select { dataset, pattern, .. }
        | Query::Construct { dataset, pattern, .. }
        | Query::Describe { dataset, pattern, .. }
        | Query::Ask { dataset, pattern, .. } => (dataset, pattern),
    }
}

pub fn analyze(query: &Query) -> Analysis {
    let (dataset, pattern) = parts(query);
    let mut analysis = Analysis { has_dataset: dataset.is_some(), ..Analysis::default() };
    if let Some(dataset) = dataset {
        for graph in dataset.default.iter().chain(dataset.named.iter().flatten()) {
            analysis.graphs.insert(graph.as_str().to_owned());
        }
    }
    // CONSTRUCT template terms are not looked up: making new triples is what a template is for.
    walk(pattern, false, &mut analysis);
    analysis
}

fn term(t: &TermPattern, remote: bool, a: &mut Analysis) {
    if let TermPattern::NamedNode(n) = t {
        insert(n, remote, a);
    }
}
fn insert(n: &NamedNode, remote: bool, a: &mut Analysis) {
    let set = if remote { &mut a.remote_terms } else { &mut a.terms };
    set.insert(n.as_str().to_owned());
}

fn path(p: &PropertyPathExpression, remote: bool, a: &mut Analysis) {
    match p {
        PropertyPathExpression::NamedNode(n) => insert(n, remote, a),
        PropertyPathExpression::Reverse(p)
        | PropertyPathExpression::ZeroOrMore(p)
        | PropertyPathExpression::OneOrMore(p)
        | PropertyPathExpression::ZeroOrOne(p) => path(p, remote, a),
        PropertyPathExpression::Sequence(x, y) | PropertyPathExpression::Alternative(x, y) => {
            path(x, remote, a);
            path(y, remote, a);
        }
        PropertyPathExpression::NegatedPropertySet(nodes) => {
            for n in nodes {
                insert(n, remote, a);
            }
        }
    }
}

fn expression(e: &Expression, remote: bool, a: &mut Analysis) {
    match e {
        Expression::NamedNode(n) => insert(n, remote, a),
        Expression::Exists(g) => walk(g, remote, a),
        Expression::Or(x, y)
        | Expression::And(x, y)
        | Expression::Equal(x, y)
        | Expression::SameTerm(x, y)
        | Expression::Greater(x, y)
        | Expression::GreaterOrEqual(x, y)
        | Expression::Less(x, y)
        | Expression::LessOrEqual(x, y)
        | Expression::Add(x, y)
        | Expression::Subtract(x, y)
        | Expression::Multiply(x, y)
        | Expression::Divide(x, y) => {
            expression(x, remote, a);
            expression(y, remote, a);
        }
        Expression::UnaryPlus(x) | Expression::UnaryMinus(x) | Expression::Not(x) => {
            expression(x, remote, a)
        }
        Expression::In(x, list) => {
            expression(x, remote, a);
            for item in list {
                expression(item, remote, a);
            }
        }
        Expression::If(x, y, z) => {
            expression(x, remote, a);
            expression(y, remote, a);
            expression(z, remote, a);
        }
        // Function IRIs (custom functions) are not data terms; only their arguments are.
        Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
            for item in list {
                expression(item, remote, a);
            }
        }
        Expression::Literal(_) | Expression::Variable(_) | Expression::Bound(_) => {}
    }
}

fn walk(g: &GraphPattern, remote: bool, a: &mut Analysis) {
    match g {
        GraphPattern::Bgp { patterns } => bgp(patterns, remote, a),
        GraphPattern::Path { subject, path: p, object } => {
            term(subject, remote, a);
            term(object, remote, a);
            path(p, remote, a);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            walk(left, remote, a);
            walk(right, remote, a);
        }
        GraphPattern::LeftJoin { left, right, expression: e } => {
            walk(left, remote, a);
            walk(right, remote, a);
            if let Some(e) = e {
                expression(e, remote, a);
            }
        }
        GraphPattern::Filter { expr, inner } => {
            expression(expr, remote, a);
            walk(inner, remote, a);
        }
        GraphPattern::Graph { name, inner } => {
            if let NamedNodePattern::NamedNode(n) = name
                && !remote
            {
                a.graphs.insert(n.as_str().to_owned());
            }
            walk(inner, remote, a);
        }
        GraphPattern::Extend { inner, expression: e, .. } => {
            walk(inner, remote, a);
            expression(e, remote, a);
        }
        GraphPattern::Values { bindings, .. } => {
            for row in bindings {
                for value in row.iter().flatten() {
                    if let GroundTerm::NamedNode(n) = value {
                        insert(n, remote, a);
                    }
                }
            }
        }
        GraphPattern::OrderBy { inner, expression: order } => {
            walk(inner, remote, a);
            for o in order {
                match o {
                    OrderExpression::Asc(e) | OrderExpression::Desc(e) => expression(e, remote, a),
                }
            }
        }
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => walk(inner, remote, a),
        GraphPattern::Group { inner, aggregates, .. } => {
            walk(inner, remote, a);
            for (_, aggregate) in aggregates {
                if let AggregateExpression::FunctionCall { expr, .. } = aggregate {
                    expression(expr, remote, a);
                }
            }
        }
        GraphPattern::Service { name, inner, .. } => {
            match name {
                NamedNodePattern::NamedNode(n) => {
                    a.services.insert(n.as_str().to_owned());
                }
                NamedNodePattern::Variable(_) => a.service_variable = true,
            }
            walk(inner, true, a);
        }
        // Variants behind optional parser features (for example LATERAL).
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// Collect terms of a basic graph pattern, recognising property-function call sites. A list
/// `(a b)` in the parsed algebra is a chain of blank nodes with `rdf:first` / `rdf:rest`; those
/// helper triples are skipped, and the list items are counted against the function.
fn bgp(patterns: &[TriplePattern], remote: bool, a: &mut Analysis) {
    let mut first: BTreeMap<String, &TermPattern> = BTreeMap::new();
    let mut rest: BTreeMap<String, &TermPattern> = BTreeMap::new();
    for t in patterns {
        if let (TermPattern::BlankNode(b), NamedNodePattern::NamedNode(p)) =
            (&t.subject, &t.predicate)
        {
            match p.as_str() {
                RDF_FIRST => {
                    first.insert(b.as_str().to_owned(), &t.object);
                }
                RDF_REST => {
                    rest.insert(b.as_str().to_owned(), &t.object);
                }
                _ => {}
            }
        }
    }
    let list = |head: &TermPattern| -> Option<Vec<&TermPattern>> {
        let mut items = Vec::new();
        let mut node = head;
        for _ in 0..64 {
            match node {
                TermPattern::NamedNode(n) if n.as_str() == RDF_NIL => return Some(items),
                TermPattern::BlankNode(b) => {
                    items.push(*first.get(b.as_str())?);
                    node = rest.get(b.as_str())?;
                }
                _ => return None,
            }
        }
        None
    };
    for t in patterns {
        let NamedNodePattern::NamedNode(p) = &t.predicate else {
            term(&t.subject, remote, a);
            term(&t.object, remote, a);
            continue;
        };
        let helper = matches!(p.as_str(), RDF_FIRST | RDF_REST)
            && matches!(t.subject, TermPattern::BlankNode(_));
        if helper {
            term(&t.object, remote, a);
            continue;
        }
        insert(p, remote, a);
        term(&t.subject, remote, a);
        term(&t.object, remote, a);
        if p.as_str().starts_with("urn:rdftool:") && !remote {
            let inputs = list(&t.object);
            let outputs = list(&t.subject);
            let mut variable_inputs = Vec::new();
            match &inputs {
                Some(items) => {
                    for item in items {
                        if let TermPattern::Variable(v) = item {
                            variable_inputs.push(v.to_string());
                        }
                    }
                }
                None => {
                    if let TermPattern::Variable(v) = &t.object {
                        variable_inputs.push(v.to_string());
                    }
                }
            }
            a.functions.push(FunctionCall {
                iri: p.as_str().to_owned(),
                inputs: inputs.map(|i| i.len()),
                outputs: outputs.map(|o| o.len()),
                variable_inputs,
            });
        }
    }
}

/// Give the query a dataset. `default` is what plain triple patterns read (merged); `named` is
/// what `GRAPH` patterns can reach. Callers pass every graph as `named`, so `GRAPH <g>` keeps
/// working for a graph that is not part of the default selection.
pub fn with_dataset(
    mut query: Query,
    default: &[String],
    named: &[String],
) -> Result<Query, String> {
    let nodes = |graphs: &[String]| -> Result<Vec<NamedNode>, String> {
        graphs
            .iter()
            .map(|g| NamedNode::new(g.clone()).map_err(|e| format!("invalid graph IRI {g}: {e}")))
            .collect()
    };
    let dataset = Some(QueryDataset { default: nodes(default)?, named: Some(nodes(named)?) });
    match &mut query {
        Query::Select { dataset: d, .. }
        | Query::Construct { dataset: d, .. }
        | Query::Describe { dataset: d, .. }
        | Query::Ask { dataset: d, .. } => *d = dataset,
    }
    Ok(query)
}

// ----- evaluating property functions ahead of a dataset clause ------------------------------------
//
// The engine evaluates a property function once per call site, before the query, and joins its
// rows as a VALUES table; it refuses to do so when the query has FROM / FROM NAMED (ADR 0003 left
// graph context for functions to a later design). Because the table does not depend on the active
// graph, this server can do the same steps itself: run each call site alone (no FROM), put its rows
// in VALUES, then add the dataset. The result is what the engine would compute.

use spargebra::term::{Literal, Variable};

/// One call site: `(outputs) <iri> (inputs)`.
#[derive(Clone, Debug)]
pub struct Site {
    pub iri: String,
    pub outputs: Vec<Variable>,
    pub inputs: Vec<TermPattern>,
}

impl Site {
    /// A query that runs just this call, without any dataset clause.
    pub fn query(&self) -> String {
        let outputs: Vec<String> = self.outputs.iter().map(ToString::to_string).collect();
        let inputs: Vec<String> = self.inputs.iter().map(ToString::to_string).collect();
        format!(
            "SELECT {} WHERE {{ ({}) <{}> ({}) }}",
            if outputs.is_empty() { "*".to_owned() } else { outputs.join(" ") },
            outputs.join(" "),
            self.iri,
            inputs.join(" ")
        )
    }
}

fn list_items<'a>(
    head: &'a TermPattern,
    first: &BTreeMap<String, &'a TermPattern>,
    rest: &BTreeMap<String, &'a TermPattern>,
    nodes: &mut BTreeSet<String>,
) -> Option<Vec<&'a TermPattern>> {
    let mut items = Vec::new();
    let mut node = head;
    for _ in 0..64 {
        match node {
            TermPattern::NamedNode(n) if n.as_str() == RDF_NIL => return Some(items),
            TermPattern::BlankNode(b) if first.contains_key(b.as_str()) => {
                nodes.insert(b.as_str().to_owned());
                items.push(*first.get(b.as_str())?);
                node = rest.get(b.as_str())?;
            }
            _ => return None,
        }
    }
    None
}

/// Split a BGP into its function call sites and the remaining patterns.
fn split_bgp(
    patterns: &[TriplePattern],
    is_function: &dyn Fn(&str) -> bool,
) -> Result<(Vec<Site>, Vec<TriplePattern>), String> {
    let mut first = BTreeMap::new();
    let mut rest = BTreeMap::new();
    for t in patterns {
        if let (TermPattern::BlankNode(b), NamedNodePattern::NamedNode(p)) =
            (&t.subject, &t.predicate)
        {
            if p.as_str() == RDF_FIRST {
                first.insert(b.as_str().to_owned(), &t.object);
            } else if p.as_str() == RDF_REST {
                rest.insert(b.as_str().to_owned(), &t.object);
            }
        }
    }
    let mut sites = Vec::new();
    let mut used_nodes = BTreeSet::new();
    let mut calls = BTreeSet::new();
    for (index, t) in patterns.iter().enumerate() {
        let NamedNodePattern::NamedNode(p) = &t.predicate else { continue };
        if !is_function(p.as_str()) {
            continue;
        }
        let outputs = match list_items(&t.subject, &first, &rest, &mut used_nodes) {
            Some(items) => items,
            None => vec![&t.subject],
        };
        let inputs = match list_items(&t.object, &first, &rest, &mut used_nodes) {
            Some(items) => items,
            None => vec![&t.object],
        };
        let mut variables = Vec::new();
        for o in outputs {
            match o {
                TermPattern::Variable(v) => variables.push(v.clone()),
                other => {
                    return Err(format!(
                        "outputs of <{}> must be variables, got {other}",
                        p.as_str()
                    ));
                }
            }
        }
        for i in &inputs {
            if matches!(i, TermPattern::Variable(_) | TermPattern::BlankNode(_)) {
                return Err(format!("<{}> needs constant inputs, got {i}", p.as_str()));
            }
        }
        sites.push(Site {
            iri: p.as_str().to_owned(),
            outputs: variables,
            inputs: inputs.into_iter().cloned().collect(),
        });
        calls.insert(index);
    }
    let remaining = patterns
        .iter()
        .enumerate()
        .filter(|(index, t)| {
            !calls.contains(index)
                && !matches!(&t.subject, TermPattern::BlankNode(b) if used_nodes.contains(b.as_str()))
        })
        .map(|(_, t)| t.clone())
        .collect();
    Ok((sites, remaining))
}

/// Apply `f` to every BGP outside SERVICE, in a fixed order.
fn each_bgp(
    g: &mut GraphPattern,
    f: &mut dyn FnMut(&mut GraphPattern) -> Result<(), String>,
) -> Result<(), String> {
    match g {
        GraphPattern::Bgp { .. } => f(g),
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            each_bgp(left, f)?;
            each_bgp(right, f)
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            each_bgp(left, f)?;
            each_bgp(right, f)?;
            if let Some(e) = expression {
                each_expression(e, f)?;
            }
            Ok(())
        }
        GraphPattern::Filter { expr, inner } => {
            each_expression(expr, f)?;
            each_bgp(inner, f)
        }
        GraphPattern::Extend { inner, expression, .. } => {
            each_bgp(inner, f)?;
            each_expression(expression, f)
        }
        GraphPattern::Graph { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. } => each_bgp(inner, f),
        // A SERVICE block is evaluated elsewhere; property functions there are not ours.
        GraphPattern::Service { .. } | GraphPattern::Path { .. } | GraphPattern::Values { .. } => {
            Ok(())
        }
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

fn each_expression(
    e: &mut Expression,
    f: &mut dyn FnMut(&mut GraphPattern) -> Result<(), String>,
) -> Result<(), String> {
    match e {
        Expression::Exists(g) => each_bgp(g, f),
        Expression::Or(x, y)
        | Expression::And(x, y)
        | Expression::Equal(x, y)
        | Expression::SameTerm(x, y)
        | Expression::Greater(x, y)
        | Expression::GreaterOrEqual(x, y)
        | Expression::Less(x, y)
        | Expression::LessOrEqual(x, y)
        | Expression::Add(x, y)
        | Expression::Subtract(x, y)
        | Expression::Multiply(x, y)
        | Expression::Divide(x, y) => {
            each_expression(x, f)?;
            each_expression(y, f)
        }
        Expression::UnaryPlus(x) | Expression::UnaryMinus(x) | Expression::Not(x) => {
            each_expression(x, f)
        }
        Expression::In(x, list) => {
            each_expression(x, f)?;
            list.iter_mut().try_for_each(|i| each_expression(i, f))
        }
        Expression::If(x, y, z) => {
            each_expression(x, f)?;
            each_expression(y, f)?;
            each_expression(z, f)
        }
        Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
            list.iter_mut().try_for_each(|i| each_expression(i, f))
        }
        _ => Ok(()),
    }
}

fn pattern_mut(query: &mut Query) -> &mut GraphPattern {
    match query {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    }
}

/// Every property-function call site in the query, in the order `substitute` replaces them.
pub fn sites(query: &Query, is_function: &dyn Fn(&str) -> bool) -> Result<Vec<Site>, String> {
    let mut copy = query.clone();
    let mut found = Vec::new();
    each_bgp(pattern_mut(&mut copy), &mut |g| {
        if let GraphPattern::Bgp { patterns } = g {
            found.extend(split_bgp(patterns, is_function)?.0);
        }
        Ok(())
    })?;
    Ok(found)
}

/// A term from a SPARQL JSON result, for a VALUES table.
pub fn ground(value: &serde_json::Value) -> Result<GroundTerm, String> {
    let text = value["value"].as_str().unwrap_or_default();
    match value["type"].as_str().unwrap_or_default() {
        "uri" => NamedNode::new(text).map(GroundTerm::NamedNode).map_err(|e| e.to_string()),
        "literal" | "typed-literal" => {
            if let Some(lang) = value.get("xml:lang").and_then(|l| l.as_str()) {
                Literal::new_language_tagged_literal(text, lang)
                    .map(GroundTerm::Literal)
                    .map_err(|e| e.to_string())
            } else if let Some(dt) = value["datatype"].as_str() {
                Ok(GroundTerm::Literal(Literal::new_typed_literal(
                    text,
                    NamedNode::new(dt).map_err(|e| e.to_string())?,
                )))
            } else {
                Ok(GroundTerm::Literal(Literal::new_simple_literal(text)))
            }
        }
        other => {
            Err(format!("a property function returned a {other}, which cannot be put in VALUES"))
        }
    }
}

/// Replace each call site with its rows, given in the order of `sites`.
pub fn substitute(
    mut query: Query,
    is_function: &dyn Fn(&str) -> bool,
    mut tables: std::collections::VecDeque<Vec<Vec<Option<GroundTerm>>>>,
) -> Result<Query, String> {
    each_bgp(pattern_mut(&mut query), &mut |g| {
        let GraphPattern::Bgp { patterns } = g else { return Ok(()) };
        let (sites, remaining) = split_bgp(patterns, is_function)?;
        if sites.is_empty() {
            return Ok(());
        }
        let mut replacement = GraphPattern::Bgp { patterns: remaining };
        for site in sites {
            let bindings = tables.pop_front().ok_or("missing property function result")?;
            let values = GraphPattern::Values { variables: site.outputs, bindings };
            replacement =
                GraphPattern::Join { left: Box::new(values), right: Box::new(replacement) };
        }
        *g = replacement;
        Ok(())
    })?;
    Ok(query)
}

// ----- writing a query back out ---------------------------------------------------------------------
//
// spargebra 0.4.8 prints a FILTER without braces of its own. Where the printed filter shares a group
// with more patterns (the left side of a join, OPTIONAL or MINUS, the pattern under a BIND) or is the
// whole right side of an OPTIONAL (where it would become the OPTIONAL's condition), re-parsing moves it
// to a wider scope and the query means something else. Before printing, each such filter is put back in
// a group of its own. (A sub-select would also print in braces, but is not the same thing inside GRAPH.)

/// The query as SPARQL text, safe to parse again with the same meaning.
pub fn to_sparql(mut query: Query) -> String {
    brace(pattern_mut(&mut query), false);
    query.to_string()
}

fn brace(g: &mut GraphPattern, exposed: bool) {
    match g {
        GraphPattern::Filter { inner, expr } => {
            brace(inner, false);
            brace_expression(expr);
            if exposed {
                // spargebra prints a FILTER on the right of a join inside braces, and an empty
                // pattern prints as nothing, so this prints as `{ P FILTER(f) }`: the group itself.
                let filter = std::mem::take(g);
                *g = GraphPattern::Join {
                    left: Box::new(GraphPattern::default()),
                    right: Box::new(filter),
                };
            }
        }
        GraphPattern::Join { left, right } | GraphPattern::Minus { left, right } => {
            brace(left, true);
            brace(right, false);
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            brace(left, true);
            // `OPTIONAL { P FILTER(f) }` parses as the OPTIONAL's own condition, so a filtered group
            // standing alone on the right must keep braces, condition or not.
            brace(right, true);
            if let Some(e) = expression {
                brace_expression(e);
            }
        }
        GraphPattern::Extend { inner, expression, .. } => {
            brace(inner, true);
            brace_expression(expression);
        }
        GraphPattern::Union { left, right } => {
            brace(left, false);
            brace(right, false);
        }
        GraphPattern::Graph { inner, .. }
        | GraphPattern::Service { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Group { inner, .. } => brace(inner, false),
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn brace_expression(e: &mut Expression) {
    match e {
        Expression::Exists(g) => brace(g, false),
        Expression::Or(x, y)
        | Expression::And(x, y)
        | Expression::Equal(x, y)
        | Expression::SameTerm(x, y)
        | Expression::Greater(x, y)
        | Expression::GreaterOrEqual(x, y)
        | Expression::Less(x, y)
        | Expression::LessOrEqual(x, y)
        | Expression::Add(x, y)
        | Expression::Subtract(x, y)
        | Expression::Multiply(x, y)
        | Expression::Divide(x, y) => {
            brace_expression(x);
            brace_expression(y);
        }
        Expression::UnaryPlus(x) | Expression::UnaryMinus(x) | Expression::Not(x) => {
            brace_expression(x)
        }
        Expression::In(x, list) => {
            brace_expression(x);
            list.iter_mut().for_each(brace_expression);
        }
        Expression::If(x, y, z) => {
            brace_expression(x);
            brace_expression(y);
            brace_expression(z);
        }
        Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
            list.iter_mut().for_each(brace_expression)
        }
        _ => {}
    }
}
