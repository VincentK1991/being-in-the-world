//! Explicit local-only query profile, including SILENT network operations.
use anyhow::{Result, ensure};
use spargebra::{
    GraphUpdateOperation, Query, Update,
    algebra::{AggregateExpression, Expression as E, GraphPattern as G, OrderExpression},
};
fn expression(e: &E) -> bool {
    match e {
        E::Exists(g) => network(g),
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
        | E::Divide(a, b) => expression(a) || expression(b),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expression(a),
        E::In(a, args) => expression(a) || args.iter().any(expression),
        E::If(a, b, c) => expression(a) || expression(b) || expression(c),
        E::Coalesce(args) | E::FunctionCall(_, args) => args.iter().any(expression),
        _ => false,
    }
}
fn network(g: &G) -> bool {
    match g {
        G::Service { .. } => true,
        G::Join { left, right }
        | G::Union { left, right }
        | G::Minus { left, right }
        | G::Lateral { left, right } => network(left) || network(right),
        G::LeftJoin { left, right, expression: e } => {
            network(left) || network(right) || e.as_ref().is_some_and(expression)
        }
        G::Filter { inner, expr } => network(inner) || expression(expr),
        G::Extend { inner, expression: e, .. } => network(inner) || expression(e),
        G::Graph { inner, .. }
        | G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. } => network(inner),
        G::OrderBy { inner, expression: order } => {
            network(inner)
                || order.iter().any(|order| match order {
                    OrderExpression::Asc(e) | OrderExpression::Desc(e) => expression(e),
                })
        }
        G::Group { inner, aggregates, .. } => {
            network(inner)
                || aggregates.iter().any(|(_, a)| match a {
                    AggregateExpression::FunctionCall { expr, .. } => expression(expr),
                    _ => false,
                })
        }
        _ => false,
    }
}
pub fn query(query: &Query) -> Result<()> {
    let pattern = match query {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    };
    ensure!(!network(pattern), "SERVICE is disabled in the local-only profile");
    Ok(())
}
pub fn update(update: &Update) -> Result<()> {
    for operation in &update.operations {
        ensure!(
            !matches!(operation, GraphUpdateOperation::Load { .. }),
            "LOAD is disabled in the local-only profile; use ingest"
        );
        if let GraphUpdateOperation::DeleteInsert { pattern, .. } = operation {
            ensure!(!network(pattern), "SERVICE is disabled in the local-only profile");
        }
    }
    Ok(())
}
