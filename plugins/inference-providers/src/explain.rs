//! "Why was this triple deduced?" for SPARQL rules. The CONSTRUCT template is matched against the
//! triple, which fixes some variables; the rule's WHERE pattern is then solved with those variables
//! fixed, and each solution's triple patterns are instantiated into the premises it relied on.
use anyhow::{Context, Result, ensure};
use rdf_store::{
    engine::Dataset,
    oxigraph::{
        model::{GraphName, NamedOrBlankNode, Quad, Term, Triple},
        sparql::{CancellationToken, QueryResults, SparqlEvaluator},
    },
};
use serde_json::{Value, json};
use spargebra::{
    Query,
    algebra::GraphPattern as G,
    term::{GroundTerm, NamedNodePattern, TermPattern as T, TriplePattern, Variable},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Binding = HashMap<Variable, Term>;
const MAX_SOLUTIONS: usize = 10;
const MAX_PREMISES: usize = 50;
const MAX_DERIVATIONS: usize = 20;

fn ground(term: &Term) -> Option<GroundTerm> {
    match term {
        Term::NamedNode(n) => Some(GroundTerm::NamedNode(n.clone())),
        Term::Literal(l) => Some(GroundTerm::Literal(l.clone())),
        _ => None,
    }
}
fn bind(pattern: &T, actual: &Term, binding: &mut Binding) -> bool {
    match pattern {
        T::Variable(v) => match binding.get(v) {
            Some(existing) => existing == actual,
            None => {
                binding.insert(v.clone(), actual.clone());
                true
            }
        },
        T::NamedNode(n) => matches!(actual, Term::NamedNode(a) if a == n),
        T::Literal(l) => matches!(actual, Term::Literal(a) if a == l),
        // A template blank node mints a fresh node per solution, so it can never equal the target.
        T::BlankNode(_) => false,
    }
}
/// The variable assignments under which `template` produces `target`, if it can.
fn unify(template: &TriplePattern, target: &Triple) -> Option<Binding> {
    let mut binding = Binding::new();
    let predicate = Term::NamedNode(target.predicate.clone());
    let ok = bind(&template.subject, &Term::from(target.subject.clone()), &mut binding)
        && match &template.predicate {
            NamedNodePattern::NamedNode(n) => *n == target.predicate,
            NamedNodePattern::Variable(v) => {
                bind(&T::Variable(v.clone()), &predicate, &mut binding)
            }
        }
        && bind(&template.object, &target.object, &mut binding);
    ok.then_some(binding)
}
/// Solutions of `pattern` that agree with `fixed`.
fn solutions(
    store: &Dataset,
    pattern: &G,
    fixed: &Binding,
    cap: usize,
    cancel: &CancellationToken,
) -> Result<Vec<Binding>> {
    rdf_store::work_budget::check(store.store(), pattern, fixed, 1_000_000)?;
    let mut variables = Vec::new();
    let mut row = Vec::new();
    for (variable, term) in fixed {
        let Some(value) = ground(term) else { return Ok(Vec::new()) };
        variables.push(variable.clone());
        row.push(Some(value));
    }
    let joined = G::Join {
        left: Box::new(G::Values { variables, bindings: vec![row] }),
        right: Box::new(pattern.clone()),
    };
    let query = Query::Select { dataset: None, pattern: joined, base_iri: None };
    let QueryResults::Solutions(iterator) = SparqlEvaluator::new()
        .with_cancellation_token(cancel.clone())
        .for_query(query)
        .on_store(store.store())
        .execute()?
    else {
        anyhow::bail!("expected solutions");
    };
    let mut found = Vec::new();
    for solution in iterator.take(cap) {
        let solution = solution?;
        found.push(solution.iter().map(|(v, t)| (v.clone(), t.clone())).collect());
    }
    Ok(found)
}
fn instantiate(pattern: &TriplePattern, binding: &Binding) -> Option<Triple> {
    let resolve = |term: &T| -> Option<Term> {
        match term {
            T::NamedNode(n) => Some(Term::NamedNode(n.clone())),
            T::Literal(l) => Some(Term::Literal(l.clone())),
            T::Variable(v) => binding.get(v).cloned(),
            T::BlankNode(_) => None,
        }
    };
    let subject = match resolve(&pattern.subject)? {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
        _ => return None,
    };
    let predicate = match &pattern.predicate {
        NamedNodePattern::NamedNode(n) => n.clone(),
        NamedNodePattern::Variable(v) => match binding.get(v)? {
            Term::NamedNode(n) => n.clone(),
            _ => return None,
        },
    };
    Some(Triple::new(subject, predicate, resolve(&pattern.object)?))
}
/// What `binding` relied on in `pattern`: every triple pattern that instantiates to a stored triple.
/// Inside an aggregate the group's members are enumerated, because the aggregate relied on all of them.
fn premises(
    store: &Dataset,
    pattern: &G,
    binding: &Binding,
    found: &mut BTreeSet<String>,
    aggregate: &mut bool,
    incomplete: &mut bool,
    cancel: &CancellationToken,
) -> Result<()> {
    if found.len() >= MAX_PREMISES {
        *incomplete = true;
        return Ok(());
    }
    match pattern {
        G::Bgp { patterns } => {
            for pattern in patterns {
                if let Some(triple) = instantiate(pattern, binding) {
                    let quad = Quad::new(
                        triple.subject.clone(),
                        triple.predicate.clone(),
                        triple.object.clone(),
                        GraphName::DefaultGraph,
                    );
                    if store.store().contains(&quad)? {
                        found.insert(format!("{triple} ."));
                    }
                }
            }
        }
        // The algebra walker has no branch-level execution trace. Never claim
        // premises from optional, alternative, negative or correlated branches.
        G::LeftJoin { .. } | G::Union { .. } | G::Minus { .. } | G::Lateral { .. } => {
            *incomplete = true;
        }
        G::Join { left, right } => {
            premises(store, left, binding, found, aggregate, incomplete, cancel)?;
            if !matches!(pattern, G::Minus { .. }) {
                premises(store, right, binding, found, aggregate, incomplete, cancel)?;
            }
        }
        G::Filter { inner, .. } => {
            *incomplete = true;
            premises(store, inner, binding, found, aggregate, incomplete, cancel)?;
        }
        G::Extend { inner, .. }
        | G::Project { inner, .. }
        | G::Distinct { inner }
        | G::Reduced { inner }
        | G::Slice { inner, .. }
        | G::OrderBy { inner, .. } => {
            premises(store, inner, binding, found, aggregate, incomplete, cancel)?;
        }
        G::Group { inner, variables, .. } => {
            *aggregate = true;
            let keys: Binding = variables
                .iter()
                .filter_map(|v| binding.get(v).map(|t| (v.clone(), t.clone())))
                .collect();
            let members = solutions(store, inner, &keys, MAX_PREMISES, cancel)?;
            *incomplete |= members.len() >= MAX_PREMISES;
            for member in members {
                premises(store, inner, &member, found, aggregate, incomplete, cancel)?;
            }
        }
        // Paths, graph blocks and inline data have no single triple to point at.
        G::Path { .. } | G::Graph { .. } | G::Service { .. } | G::Values { .. } => {
            *incomplete = true;
        }
    }
    Ok(())
}

/// Every way the rules produce `target` from the data in `store`.
pub fn explain(
    store: &Dataset,
    rules: &[Query],
    texts: &[String],
    target: &Triple,
    cancel: &CancellationToken,
) -> Result<Vec<Value>> {
    ensure!(rules.len() == texts.len(), "rules and rule texts differ");
    let mut found = Vec::new();
    for (index, (rule, text)) in rules.iter().zip(texts).enumerate() {
        let Query::Construct { template, pattern, .. } = rule else { continue };
        for binding in template.iter().filter_map(|t| unify(t, target)) {
            for solution in solutions(store, pattern, &binding, MAX_SOLUTIONS, cancel)
                .with_context(|| format!("solving rule {index}"))?
            {
                let mut support = BTreeSet::new();
                let (mut aggregate, mut incomplete) = (false, false);
                premises(
                    store,
                    pattern,
                    &solution,
                    &mut support,
                    &mut aggregate,
                    &mut incomplete,
                    cancel,
                )?;
                let values: BTreeMap<String, String> = solution
                    .iter()
                    .filter(|(variable, _)| {
                        !variable.as_str().starts_with("__") && !variable.as_str().starts_with("g_")
                    })
                    .map(|(variable, term)| (variable.as_str().to_owned(), term.to_string()))
                    .collect();
                found.push(json!({
                    "rule": index, "rule_text": text, "bindings": values,
                    "premises": support.into_iter().collect::<Vec<_>>(),
                    "aggregate": aggregate, "incomplete": incomplete,
                }));
                if found.len() >= MAX_DERIVATIONS {
                    return Ok(found);
                }
            }
        }
    }
    Ok(found)
}
