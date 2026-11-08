//! Bounded relation expansion on parsed algebra, never textual SPARQL replacement.
use anyhow::{Context, Result, bail, ensure};
use rdf_plugin_api::{PropertyDescriptor, PropertyFunction, RdfValue, ReadContext, RowSink};
use rdf_store::{
    engine::{Dataset, write_results},
    oxigraph::{
        io::RdfFormat,
        model::Literal,
        sparql::{QueryResults, results::QueryResultsFormat},
    },
};
use spargebra::{
    GraphUpdateOperation, Query, SparqlParser,
    algebra::{
        AggregateExpression, Expression as E, Function, GraphPattern as G, OrderExpression,
        PropertyPathExpression as P,
    },
    term::{GroundTerm, NamedNode, NamedNodePattern, TermPattern as T, TriplePattern, Variable},
};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

pub type Registry = BTreeMap<String, (PropertyDescriptor, Arc<dyn PropertyFunction>)>;
const PREFIX: &str = "urn:rdftool:property:";
const FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
const MAX_ROWS: usize = 10_000;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub struct DatasetContext<'a>(pub &'a Dataset);
impl ReadContext for DatasetContext<'_> {
    fn query_json(&self, query: &str) -> Result<serde_json::Value> {
        let result = self.0.query(query)?;
        ensure!(!matches!(result, QueryResults::Graph(_)), "context requires SELECT or ASK");
        let mut bytes = Vec::new();
        write_results(result, QueryResultsFormat::Json, RdfFormat::Turtle, &mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn special(registry: &Registry, iri: &str) -> bool {
    registry.contains_key(iri) || iri.starts_with(PREFIX) || iri.starts_with("urn:rdftool:search:")
}
fn predicate(triple: &TriplePattern) -> Option<&str> {
    match &triple.predicate {
        NamedNodePattern::NamedNode(node) => Some(node.as_str()),
        _ => None,
    }
}
fn constant(term: &T) -> Result<GroundTerm> {
    match term {
        T::NamedNode(n) => Ok(n.clone().into()),
        T::Literal(l) => Ok(l.clone().into()),
        _ => bail!(
            "property inputs must be constants; correlated variables and blank nodes are unsupported"
        ),
    }
}
fn sdk(term: GroundTerm) -> RdfValue {
    match term {
        GroundTerm::NamedNode(n) => RdfValue::Iri(n.into_string()),
        GroundTerm::Literal(l) => RdfValue::Literal {
            value: l.value().into(),
            datatype: l.datatype().as_str().into(),
            language: l.language().map(str::to_owned),
        },
    }
}
fn ground(term: RdfValue) -> Result<GroundTerm> {
    Ok(match term {
        RdfValue::Iri(iri) => NamedNode::new(iri)?.into(),
        RdfValue::Literal { value, datatype, language } => {
            if let Some(language) = language {
                ensure!(
                    datatype == "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                    "language literal must have rdf:langString datatype"
                );
                Literal::new_language_tagged_literal(value, language)?.into()
            } else {
                ensure!(
                    datatype != "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                    "rdf:langString requires a language"
                );
                Literal::new_typed_literal(value, NamedNode::new(datatype)?).into()
            }
        }
    })
}
fn bytes(term: &RdfValue) -> usize {
    match term {
        RdfValue::Iri(iri) => iri.len(),
        RdfValue::Literal { value, datatype, language } => {
            value.len() + datatype.len() + language.as_ref().map_or(0, String::len)
        }
    }
}

/// A query-syntax list cell: a blank node, or a variable that heads `rdf:first` in this same BGP.
/// Federation engines rewrite blank nodes to variables when they forward a sub-pattern, so a property
/// call survives being sent to another endpoint. Only the query's own patterns are read, never stored data.
fn is_cell(term: &T, patterns: &[TriplePattern]) -> bool {
    match term {
        T::BlankNode(_) => true,
        T::Variable(_) => {
            patterns.iter().any(|p| p.subject == *term && predicate(p) == Some(FIRST))
        }
        _ => false,
    }
}

/// Read only query-syntax collections in the same BGP. Never traverse stored RDF lists.
fn arguments(
    term: &T,
    patterns: &[TriplePattern],
    used: &mut HashSet<usize>,
    cells: &mut HashSet<T>,
) -> Result<Vec<T>> {
    if matches!(term, T::NamedNode(n) if n.as_str() == NIL) {
        return Ok(vec![]);
    }
    if !is_cell(term, patterns) {
        return Ok(vec![term.clone()]);
    }
    let mut cursor = term.clone();
    let mut result = Vec::new();
    loop {
        if matches!(&cursor, T::NamedNode(n) if n.as_str() == NIL) {
            break;
        }
        ensure!(
            is_cell(&cursor, patterns) && cells.insert(cursor.clone()),
            "property lists must be finite, unshared query-syntax collections"
        );
        ensure!(result.len() < 32, "property argument list exceeds 32 terms");
        let find = |iri| -> Result<usize> {
            let matches: Vec<_> = patterns
                .iter()
                .enumerate()
                .filter(|(_, p)| p.subject == cursor && predicate(p) == Some(iri))
                .map(|(i, _)| i)
                .collect();
            ensure!(matches.len() == 1, "malformed property argument list");
            Ok(matches[0])
        };
        let first = find(FIRST)?;
        let rest = find(REST)?;
        ensure!(used.insert(first) && used.insert(rest), "shared property list is unsupported");
        result.push(patterns[first].object.clone());
        cursor = patterns[rest].object.clone();
    }
    Ok(result)
}

#[derive(Default)]
struct Budget {
    rows: usize,
    bytes: usize,
    calls: usize,
}
struct Table<'a> {
    output: Vec<T>,
    variables: Vec<Variable>,
    bindings: Vec<Vec<Option<GroundTerm>>>,
    budget: &'a mut Budget,
    failed: bool,
}
impl Table<'_> {
    fn push(&mut self, row: Vec<RdfValue>) -> Result<()> {
        ensure!(row.len() == self.output.len(), "property returned wrong output arity");
        self.budget.rows += 1;
        self.budget.bytes = self.budget.bytes.saturating_add(row.iter().map(bytes).sum::<usize>());
        ensure!(
            self.budget.rows <= MAX_ROWS && self.budget.bytes <= MAX_BYTES,
            "property query exceeds 10000 rows or 8 MiB; LIMIT does not bound property expansion"
        );
        let terms = row.into_iter().map(ground).collect::<Result<Vec<_>>>()?;
        let mut values = BTreeMap::new();
        for (target, value) in self.output.iter().zip(terms) {
            if let T::Variable(variable) = target {
                if let Some(previous) = values.insert(variable.clone(), value.clone())
                    && previous != value
                {
                    return Ok(());
                }
            } else if constant(target)? != value {
                return Ok(());
            }
        }
        self.bindings.push(self.variables.iter().map(|v| values.get(v).cloned()).collect());
        Ok(())
    }
}
impl RowSink for Table<'_> {
    fn emit(&mut self, row: Vec<RdfValue>) -> Result<()> {
        ensure!(!self.failed, "property row sink already failed");
        let result = self.push(row);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

struct Rewrite<'a> {
    registry: &'a Registry,
    context: Option<&'a dyn ReadContext>,
    budget: Budget,
}
impl Rewrite<'_> {
    fn bgp(&mut self, patterns: &[TriplePattern], allowed: bool) -> Result<G> {
        let calls: Vec<_> = patterns
            .iter()
            .enumerate()
            .filter(|(_, p)| predicate(p).is_some_and(|iri| special(self.registry, iri)))
            .collect();
        if calls.is_empty() {
            return Ok(G::Bgp { patterns: patterns.to_vec() });
        }
        ensure!(
            allowed,
            "property functions are query-only and unsupported inside GRAPH, FROM/FROM NAMED, or UPDATE"
        );
        let mut used = HashSet::new();
        let mut cells = HashSet::new();
        let mut tables = Vec::new();
        for (index, triple) in calls {
            used.insert(index);
            let iri = predicate(triple).expect("named property predicate");
            let (descriptor, function) = self
                .registry
                .get(iri)
                .with_context(|| format!("unregistered property function: {iri}"))?;
            let output = arguments(&triple.subject, patterns, &mut used, &mut cells)?;
            let input = arguments(&triple.object, patterns, &mut used, &mut cells)?;
            ensure!(
                output.len() == descriptor.output_arity && input.len() == descriptor.input_arity,
                "wrong property arity for {iri}: expected {} outputs and {} inputs",
                descriptor.output_arity,
                descriptor.input_arity
            );
            let input =
                input.iter().map(constant).map(|term| term.map(sdk)).collect::<Result<Vec<_>>>()?;
            ensure!(
                input.iter().map(bytes).sum::<usize>() <= 64 * 1024,
                "property input exceeds 64 KiB"
            );
            let mut variables = Vec::new();
            for target in &output {
                if let T::Variable(variable) = target {
                    if !variables.contains(variable) {
                        variables.push(variable.clone());
                    }
                } else {
                    constant(target)?;
                }
            }
            self.budget.calls += 1;
            ensure!(self.budget.calls <= 64, "query exceeds 64 property calls");
            let mut table = Table {
                output,
                variables,
                bindings: Vec::new(),
                budget: &mut self.budget,
                failed: false,
            };
            if let Some(context) = self.context {
                function
                    .evaluate(context, &input, &mut table)
                    .map_err(|error| anyhow::anyhow!("property {iri} failed: {error:#}"))?;
                ensure!(!table.failed, "property {iri} ignored a row sink failure");
            }
            tables.push(G::Values { variables: table.variables, bindings: table.bindings });
        }
        let mut remaining = Vec::new();
        for (index, pattern) in patterns.iter().enumerate() {
            if used.contains(&index) {
                continue;
            }
            ensure!(
                !cells.contains(&pattern.subject) && !cells.contains(&pattern.object),
                "property list cells may not be reused by ordinary triple patterns"
            );
            remaining.push(pattern.clone());
        }
        let mut result = G::Bgp { patterns: remaining };
        for table in tables {
            result = G::Join { left: Box::new(result), right: Box::new(table) };
        }
        Ok(result)
    }
    /// Federation engines push a FILTER down onto one list cell, which splits a property call's list
    /// across two groups: `{ ?c1 rdf:first ?k . FILTER(f) } ?c0 rdf:first ?p . ...`. Joining the two
    /// triple blocks back into one is equivalent when every filter only uses variables its own block
    /// binds, so the call can be read again. Only done when the result contains a property call.
    fn merged(&self, left: &G, right: &G) -> Option<G> {
        fn block(pattern: &G) -> Option<(Option<&E>, &[TriplePattern])> {
            match pattern {
                G::Bgp { patterns } => Some((None, patterns)),
                G::Filter { expr, inner } => match &**inner {
                    G::Bgp { patterns } => Some((Some(expr), patterns)),
                    _ => None,
                },
                _ => None,
            }
        }
        let (left, right) = (block(left)?, block(right)?);
        if left.0.is_none() && right.0.is_none() {
            return None;
        }
        let all: Vec<TriplePattern> = left.1.iter().chain(right.1).cloned().collect();
        all.iter()
            .any(|p| predicate(p).is_some_and(|iri| special(self.registry, iri)))
            .then_some(())?;
        let mut filters = Vec::new();
        for (filter, patterns) in [left, right] {
            if let Some(expr) = filter {
                let used = expression_variables(expr)?;
                let bound = pattern_variables(patterns);
                used.is_subset(&bound).then_some(())?;
                filters.push(expr.clone());
            }
        }
        let mut result = G::Bgp { patterns: all };
        if let Some(first) = filters.pop() {
            let expr = filters.into_iter().fold(first, |a, b| E::And(Box::new(a), Box::new(b)));
            result = G::Filter { expr, inner: Box::new(result) };
        }
        Some(result)
    }
    fn path(&self, path: &P) -> bool {
        match path {
            P::NamedNode(n) => special(self.registry, n.as_str()),
            P::Reverse(p) | P::ZeroOrMore(p) | P::OneOrMore(p) | P::ZeroOrOne(p) => self.path(p),
            P::Sequence(a, b) | P::Alternative(a, b) => self.path(a) || self.path(b),
            P::NegatedPropertySet(nodes) => {
                nodes.iter().any(|n| special(self.registry, n.as_str()))
            }
        }
    }
    fn expression(&mut self, expr: &mut E, allowed: bool) -> Result<()> {
        match expr {
            E::Exists(g) => self.pattern(g, allowed)?,
            E::Or(a, b)
            | E::And(a, b)
            | E::Equal(a, b)
            | E::SameTerm(a, b)
            | E::Greater(a, b)
            | E::GreaterOrEqual(a, b)
            | E::Less(a, b)
            | E::LessOrEqual(a, b)
            | E::Add(a, b)
            | E::Subtract(a, b)
            | E::Multiply(a, b)
            | E::Divide(a, b) => {
                self.expression(a, allowed)?;
                self.expression(b, allowed)?;
            }
            E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => self.expression(a, allowed)?,
            E::In(a, args) => {
                self.expression(a, allowed)?;
                for arg in args {
                    self.expression(arg, allowed)?;
                }
            }
            E::If(a, b, c) => {
                self.expression(a, allowed)?;
                self.expression(b, allowed)?;
                self.expression(c, allowed)?;
            }
            E::Coalesce(args) => {
                for arg in args {
                    self.expression(arg, allowed)?;
                }
            }
            E::FunctionCall(function, args) => {
                ensure!(
                    !matches!(function, Function::Custom(n) if special(self.registry, n.as_str())),
                    "property functions must be used as predicates, not scalar expression calls"
                );
                for arg in args {
                    self.expression(arg, allowed)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pattern(&mut self, pattern: &mut G, allowed: bool) -> Result<()> {
        match pattern {
            G::Bgp { patterns } => *pattern = self.bgp(patterns, allowed)?,
            G::Path { path, .. } => {
                ensure!(!self.path(path), "property functions cannot be used in property paths")
            }
            G::Join { left, right } if self.merged(left, right).is_some() => {
                *pattern = self.merged(left, right).expect("checked");
                self.pattern(pattern, allowed)?;
            }
            G::Join { left, right }
            | G::Union { left, right }
            | G::Minus { left, right }
            | G::Lateral { left, right } => {
                self.pattern(left, allowed)?;
                self.pattern(right, allowed)?;
            }
            G::LeftJoin { left, right, expression } => {
                self.pattern(left, allowed)?;
                self.pattern(right, allowed)?;
                if let Some(e) = expression {
                    self.expression(e, allowed)?;
                }
            }
            G::Filter { expr, inner } | G::Extend { expression: expr, inner, .. } => {
                self.expression(expr, allowed)?;
                self.pattern(inner, allowed)?;
            }
            G::Graph { inner, .. } | G::Service { inner, .. } => self.pattern(inner, false)?,
            G::Project { inner, .. }
            | G::Distinct { inner }
            | G::Reduced { inner }
            | G::Slice { inner, .. } => self.pattern(inner, allowed)?,
            G::OrderBy { inner, expression } => {
                self.pattern(inner, allowed)?;
                for order in expression {
                    let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = order;
                    self.expression(e, allowed)?;
                }
            }
            G::Group { inner, aggregates, .. } => {
                self.pattern(inner, allowed)?;
                for (_, aggregate) in aggregates {
                    if let AggregateExpression::FunctionCall { expr, .. } = aggregate {
                        self.expression(expr, allowed)?;
                    }
                }
            }
            G::Values { .. } => {}
        }
        Ok(())
    }
    fn query(&mut self, query: &mut Query) -> Result<()> {
        let allowed = query.dataset().is_none();
        let (Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }) = query;
        self.pattern(pattern, allowed)
    }
}
/// Variables a block of triple patterns binds.
fn pattern_variables(patterns: &[TriplePattern]) -> HashSet<Variable> {
    let mut found = HashSet::new();
    for p in patterns {
        for term in [&p.subject, &p.object] {
            if let T::Variable(v) = term {
                found.insert(v.clone());
            }
        }
        if let NamedNodePattern::Variable(v) = &p.predicate {
            found.insert(v.clone());
        }
    }
    found
}
/// Variables an expression reads; `None` when it contains something this does not analyse (EXISTS).
fn expression_variables(expr: &E) -> Option<HashSet<Variable>> {
    let mut found = HashSet::new();
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        match e {
            E::NamedNode(_) | E::Literal(_) => {}
            E::Variable(v) | E::Bound(v) => {
                found.insert(v.clone());
            }
            E::Or(a, b)
            | E::And(a, b)
            | E::Equal(a, b)
            | E::SameTerm(a, b)
            | E::Greater(a, b)
            | E::GreaterOrEqual(a, b)
            | E::Less(a, b)
            | E::LessOrEqual(a, b)
            | E::Add(a, b)
            | E::Subtract(a, b)
            | E::Multiply(a, b)
            | E::Divide(a, b) => stack.extend([&**a, &**b]),
            E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => stack.push(a),
            E::In(a, list) => {
                stack.push(a);
                stack.extend(list);
            }
            E::If(a, b, c) => stack.extend([&**a, &**b, &**c]),
            E::Coalesce(list) | E::FunctionCall(_, list) => stack.extend(list),
            E::Exists(_) => return None,
        }
    }
    Some(found)
}
pub fn prepare(mut query: Query, registry: &Registry, context: &dyn ReadContext) -> Result<Query> {
    // Validate the complete algebra before executing any trusted callbacks.
    Rewrite { registry, context: None, budget: Budget::default() }.query(&mut query.clone())?;
    Rewrite { registry, context: Some(context), budget: Budget::default() }.query(&mut query)?;
    Ok(query)
}
pub fn validate_update(update: &str, registry: &Registry) -> Result<()> {
    let mut update = SparqlParser::new().parse_update(update)?;
    let mut rewrite = Rewrite { registry, context: None, budget: Budget::default() };
    for operation in &mut update.operations {
        if let GraphUpdateOperation::DeleteInsert { pattern, .. } = operation {
            rewrite.pattern(pattern, false)?;
        }
    }
    Ok(())
}
