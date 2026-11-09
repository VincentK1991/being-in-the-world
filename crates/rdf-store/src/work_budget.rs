//! Conservative preflight for bounded snapshot and explanation queries. This may
//! reject affordable joins; it is not a replacement for cooperative cancellation.
use anyhow::{Result, bail, ensure};
use oxigraph::{
    model::{GraphName, NamedOrBlankNode, Term},
    store::Store,
};
use spargebra::{
    algebra::{Expression as E, GraphPattern as G},
    term::{NamedNodePattern, TermPattern as T, Variable},
};
use std::collections::HashMap;
pub fn check(
    store: &Store,
    pattern: &G,
    fixed: &HashMap<Variable, Term>,
    limit: usize,
) -> Result<()> {
    let cost = estimate(store, pattern, fixed, limit)?;
    ensure!(cost <= limit, "query exceeds conservative work budget ({limit}); narrow the query");
    Ok(())
}
fn estimate(store: &Store, p: &G, fixed: &HashMap<Variable, Term>, limit: usize) -> Result<usize> {
    let resolve = |t: &T| match t {
        T::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        T::Literal(l) => Some(Term::Literal(l.clone())),
        T::Variable(v) => fixed.get(v).cloned(),
        _ => None,
    };
    let cost = match p {
        G::Bgp { patterns } => {
            let mut product = 1usize;
            for t in patterns {
                let subject = resolve(&t.subject).and_then(|t| match t {
                    Term::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n)),
                    Term::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(b)),
                    _ => None,
                });
                let predicate = match &t.predicate {
                    NamedNodePattern::NamedNode(n) => Some(n.clone()),
                    NamedNodePattern::Variable(v) => match fixed.get(v) {
                        Some(Term::NamedNode(n)) => Some(n.clone()),
                        _ => None,
                    },
                };
                let object = resolve(&t.object);
                let count = store
                    .quads_for_pattern(
                        subject.as_ref().map(|s| s.as_ref()),
                        predicate.as_ref().map(|p| p.as_ref()),
                        object.as_ref().map(|o| o.as_ref()),
                        Some(GraphName::DefaultGraph.as_ref()),
                    )
                    .take(limit + 1)
                    .collect::<Result<Vec<_>, _>>()?
                    .len();
                product = product.saturating_mul(count.max(1));
                ensure!(
                    product <= limit,
                    "query exceeds conservative work budget ({limit}); narrow the query"
                );
            }
            product
        }
        G::LeftJoin { left, right, expression } => {
            ensure!(
                !expression.as_ref().is_some_and(has_exists),
                "bounded query profile does not evaluate EXISTS expressions"
            );
            estimate(store, left, fixed, limit)?
                .saturating_mul(estimate(store, right, fixed, limit)?)
        }
        G::Join { left, right } | G::Lateral { left, right } | G::Minus { left, right } => {
            estimate(store, left, fixed, limit)?
                .saturating_mul(estimate(store, right, fixed, limit)?)
        }
        G::Union { left, right } => estimate(store, left, fixed, limit)?
            .saturating_add(estimate(store, right, fixed, limit)?),
        G::Filter { expr, inner } | G::Extend { expression: expr, inner, .. } => {
            ensure!(
                !has_exists(expr),
                "bounded query profile does not evaluate EXISTS expressions"
            );
            estimate(store, inner, fixed, limit)?
        }
        G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => estimate(store, inner, fixed, limit)?,
        G::OrderBy { inner, expression } => {
            ensure!(
                !expression.iter().any(|e| match e {
                    spargebra::algebra::OrderExpression::Asc(e)
                    | spargebra::algebra::OrderExpression::Desc(e) => has_exists(e),
                }),
                "bounded query profile does not evaluate EXISTS expressions"
            );
            estimate(store, inner, fixed, limit)?
        }
        G::Group { inner, aggregates, .. } => {
            ensure!(
                !aggregates.iter().any(|(_, e)| match e {
                    spargebra::algebra::AggregateExpression::FunctionCall { expr, .. } =>
                        has_exists(expr),
                    _ => false,
                }),
                "bounded query profile does not evaluate EXISTS expressions"
            );
            estimate(store, inner, fixed, limit)?
        }
        G::Values { bindings, .. } => bindings.len().max(1),
        G::Graph { .. } | G::Service { .. } | G::Path { .. } => {
            bail!("bounded query profile does not evaluate GRAPH, SERVICE or paths")
        }
    };
    ensure!(cost <= limit, "query exceeds conservative work budget ({limit}); narrow the query");
    Ok(cost)
}
fn has_exists(e: &E) -> bool {
    match e {
        E::Exists(_) => true,
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
        | E::Divide(a, b) => has_exists(a) || has_exists(b),
        E::In(a, b) => has_exists(a) || b.iter().any(has_exists),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => has_exists(a),
        E::If(a, b, c) => has_exists(a) || has_exists(b) || has_exists(c),
        E::Coalesce(a) | E::FunctionCall(_, a) => a.iter().any(has_exists),
        _ => false,
    }
}
