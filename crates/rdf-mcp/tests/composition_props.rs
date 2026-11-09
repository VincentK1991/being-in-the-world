//! Property tests: the rewrites `sparql_query` applies keep a query's meaning, whatever its shape.
//!
//! Random queries are built from a small grammar (triples, GRAPH, OPTIONAL, UNION, FILTER, nested
//! groups and property-function calls) over random data spread across named graphs and the
//! default graph. Each property compares what this server sends with a reference built
//! independently from the same grammar, both evaluated by Oxigraph's own evaluator (`spareval`).
use oxrdf::{Dataset, GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term as RdfTerm};
use proptest::prelude::*;
use rdf_mcp::sparql;
use spareval::{QueryEvaluator, QueryResults};
use spargebra::SparqlParser;
use spargebra::term::{GroundTerm, TermPattern};
use std::collections::{BTreeSet, VecDeque};

const SPLIT: &str = "urn:rdftool:property:split";
const SEARCH: &str = "urn:rdftool:search:hybrid";
const GRAPHS: usize = 3;
const VARS: &[&str] = &["?s", "?o", "?x", "?v"];

fn iri(i: usize) -> String {
    format!("https://ex.org/i{i}")
}
fn predicate(i: usize) -> String {
    format!("https://ex.org/p{i}")
}
fn graph(i: usize) -> String {
    format!("urn:g:{i}")
}
fn literal(i: usize) -> String {
    format!("v{i}")
}
fn is_function(iri: &str) -> bool {
    iri == SPLIT || iri == SEARCH
}

#[derive(Clone, Debug)]
enum Term {
    Var(usize),
    Iri(usize),
    Lit(usize),
}
impl Term {
    fn render(&self) -> String {
        match self {
            Term::Var(v) => VARS[*v].to_owned(),
            Term::Iri(i) => format!("<{}>", iri(*i)),
            Term::Lit(l) => format!("\"{}\"", literal(*l)),
        }
    }
}

#[derive(Clone, Debug)]
enum Pat {
    Triple(Term, usize, Term),
    /// A property-function call: `(?var) split ("v1,v3" ",")`, whose rows are the items.
    Split(usize, Vec<usize>),
    Group(Vec<Pat>),
    Graph(Option<usize>, Box<Pat>),
    Optional(Box<Pat>, Box<Pat>),
    Union(Box<Pat>, Box<Pat>),
    Filter(Box<Pat>, usize, usize),
    /// `{ P BIND(<iri> AS ?bN) }`; nesting can rebind, which SPARQL rejects (such cases are skipped).
    Bind(Box<Pat>, usize, usize),
    Minus(Box<Pat>, Box<Pat>),
    /// Only in structural tests: a block for a remote endpoint.
    Service(Box<Pat>),
}

#[derive(Clone, Copy, PartialEq)]
enum Calls {
    /// Property-function syntax, as an agent writes it.
    Function,
    /// The reference: each call written by hand as the VALUES table it stands for.
    Values,
}

impl Pat {
    fn render(&self, calls: Calls) -> String {
        match self {
            Pat::Triple(s, p, o) => format!("{} <{}> {} .", s.render(), predicate(*p), o.render()),
            Pat::Split(v, items) => {
                let items: Vec<String> = items.iter().map(|i| literal(*i)).collect();
                match calls {
                    Calls::Function => {
                        format!("({}) <{SPLIT}> (\"{}\" \",\") .", VARS[*v], items.join(","))
                    }
                    Calls::Values => {
                        let rows: Vec<String> = items.iter().map(|i| format!("\"{i}\"")).collect();
                        format!("VALUES {} {{ {} }}", VARS[*v], rows.join(" "))
                    }
                }
            }
            Pat::Group(items) => {
                let inner: Vec<String> = items.iter().map(|p| p.render(calls)).collect();
                format!("{{ {} }}", inner.join(" "))
            }
            Pat::Graph(name, inner) => {
                let name = name.map(|g| format!("<{}>", graph(g))).unwrap_or_else(|| "?g".into());
                format!("GRAPH {name} {{ {} }}", inner.render(calls))
            }
            Pat::Optional(a, b) => {
                format!("{{ {{ {} }} OPTIONAL {{ {} }} }}", a.render(calls), b.render(calls))
            }
            Pat::Union(a, b) => {
                format!("{{ {{ {} }} UNION {{ {} }} }}", a.render(calls), b.render(calls))
            }
            Pat::Filter(inner, v, i) => {
                format!(
                    "{{ {} FILTER(!BOUND({var}) || {var} != <{}>) }}",
                    inner.render(calls),
                    iri(*i),
                    var = VARS[*v]
                )
            }
            Pat::Bind(inner, b, i) => {
                format!("{{ {} BIND(<{}> AS ?b{b}) }}", inner.render(calls), iri(*i))
            }
            Pat::Minus(a, b) => {
                format!("{{ {{ {} }} MINUS {{ {} }} }}", a.render(calls), b.render(calls))
            }
            Pat::Service(inner) => {
                format!("SERVICE <http://remote.example/sparql> {{ {} }}", inner.render(calls))
            }
        }
    }
    /// Calls outside SERVICE, and inside.
    fn calls(&self) -> (usize, usize) {
        match self {
            Pat::Triple(..) => (0, 0),
            Pat::Split(..) => (1, 0),
            Pat::Group(items) => {
                items.iter().map(Pat::calls).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
            }
            Pat::Graph(_, inner) | Pat::Filter(inner, ..) | Pat::Bind(inner, ..) => inner.calls(),
            Pat::Optional(a, b) | Pat::Union(a, b) | Pat::Minus(a, b) => {
                let (x, y) = (a.calls(), b.calls());
                (x.0 + y.0, x.1 + y.1)
            }
            Pat::Service(inner) => {
                let (local, remote) = inner.calls();
                (0, local + remote)
            }
        }
    }
}

fn term() -> impl Strategy<Value = Term> {
    prop_oneof![
        3 => (0..VARS.len()).prop_map(Term::Var),
        1 => (0..5usize).prop_map(Term::Iri),
        1 => (0..4usize).prop_map(Term::Lit),
    ]
}

fn pattern(with_service: bool) -> impl Strategy<Value = Pat> {
    let leaf = prop_oneof![
        4 => (term(), 0..3usize, term()).prop_map(|(s, p, o)| Pat::Triple(s, p, o)),
        1 => (0..VARS.len(), prop::collection::vec(0..4usize, 1..4)).prop_map(|(v, items)| Pat::Split(v, items)),
    ];
    leaf.prop_recursive(3, 16, 4, move |inner| {
        let mut choices = vec![
            prop::collection::vec(inner.clone(), 1..4).prop_map(Pat::Group).boxed(),
            (prop::option::of(0..GRAPHS), inner.clone())
                .prop_map(|(g, p)| Pat::Graph(g, Box::new(p)))
                .boxed(),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| Pat::Optional(Box::new(a), Box::new(b)))
                .boxed(),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| Pat::Union(Box::new(a), Box::new(b)))
                .boxed(),
            (inner.clone(), 0..VARS.len(), 0..5usize)
                .prop_map(|(p, v, i)| Pat::Filter(Box::new(p), v, i))
                .boxed(),
            (inner.clone(), 0..2usize, 0..5usize)
                .prop_map(|(p, b, i)| Pat::Bind(Box::new(p), b, i))
                .boxed(),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| Pat::Minus(Box::new(a), Box::new(b)))
                .boxed(),
        ];
        if with_service {
            choices.push(inner.prop_map(|p| Pat::Service(Box::new(p))).boxed());
        }
        prop::strategy::Union::new(choices)
    })
}

/// (subject, predicate, object, graph); `None` is the unnamed default graph.
type Fact = (usize, usize, Result<usize, usize>, Option<usize>);

fn facts() -> impl Strategy<Value = Vec<Fact>> {
    prop::collection::vec(
        (
            0..5usize,
            0..3usize,
            prop_oneof![(0..5usize).prop_map(Ok), (0..4usize).prop_map(Err)],
            prop::option::of(0..GRAPHS),
        ),
        0..30,
    )
}

fn named(text: &str) -> NamedNode {
    NamedNode::new(text).expect("valid IRI")
}
fn quad(fact: &Fact, graph_name: GraphName) -> Quad {
    let (s, p, o, _) = fact;
    let object: RdfTerm = match o {
        Ok(i) => named(&iri(*i)).into(),
        Err(l) => Literal::new_simple_literal(literal(*l)).into(),
    };
    Quad::new(NamedOrBlankNode::from(named(&iri(*s))), named(&predicate(*p)), object, graph_name)
}

/// The store as it is: named graphs and the unnamed default graph.
fn store(facts: &[Fact]) -> Dataset {
    facts
        .iter()
        .map(|f| {
            quad(
                f,
                f.3.map(|g| GraphName::from(named(&graph(g)))).unwrap_or(GraphName::DefaultGraph),
            )
        })
        .collect()
}

/// The reference for "plain patterns read the selected graphs merged": the same named graphs, and a
/// default graph that is exactly the union of the selected ones (the store's own default graph is
/// not part of it).
fn merged(facts: &[Fact], selected: &[usize]) -> Dataset {
    let mut data: Dataset = facts
        .iter()
        .filter_map(|f| f.3.map(|g| quad(f, GraphName::from(named(&graph(g))))))
        .collect();
    for f in facts {
        if let Some(g) = f.3
            && selected.contains(&g)
        {
            data.insert(&quad(f, GraphName::DefaultGraph));
        }
    }
    data
}

/// Results as a set of solutions (or the boolean, or triples), each written canonically.
fn evaluate(text: &str, data: &Dataset) -> BTreeSet<String> {
    let query = SparqlParser::new().parse_query(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let evaluator = QueryEvaluator::new();
    match evaluator.prepare(&query).execute(data).unwrap_or_else(|e| panic!("{e}\n{text}")) {
        QueryResults::Solutions(solutions) => solutions
            .map(|s| {
                let s = s.unwrap_or_else(|e| panic!("{e}"));
                let mut cells: Vec<String> = s.iter().map(|(v, t)| format!("{v}={t}")).collect();
                cells.sort();
                cells.join(" ")
            })
            .collect(),
        QueryResults::Boolean(b) => BTreeSet::from([b.to_string()]),
        QueryResults::Graph(triples) => {
            triples.map(|t| t.unwrap_or_else(|e| panic!("{e}")).to_string()).collect()
        }
    }
}

/// What the split plugin returns for one call site.
fn split_rows(site: &sparql::Site) -> Vec<Vec<Option<GroundTerm>>> {
    let text = |t: &TermPattern| match t {
        TermPattern::Literal(l) => l.value().to_owned(),
        other => panic!("constant expected, got {other}"),
    };
    let (value, separator) = (text(&site.inputs[0]), text(&site.inputs[1]));
    value
        .split(separator.as_str())
        .map(|item| {
            vec![Some(GroundTerm::Literal(spargebra::term::Literal::new_simple_literal(item)))]
        })
        .collect()
}

/// What `sparql_query` sends for a local query with `graphs` = `selected`.
fn rewritten(text: &str, selected: &[usize]) -> String {
    let (query, _) = sparql::parse(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let sites = sparql::sites(&query, &is_function).unwrap_or_else(|e| panic!("{e}"));
    let tables: VecDeque<_> = sites.iter().map(split_rows).collect();
    let query = sparql::substitute(query, &is_function, tables).unwrap_or_else(|e| panic!("{e}"));
    let default: Vec<String> = selected.iter().map(|g| graph(*g)).collect();
    let all: Vec<String> = (0..GRAPHS).map(graph).collect();
    sparql::to_sparql(sparql::with_dataset(query, &default, &all).unwrap_or_else(|e| panic!("{e}")))
}

fn selection() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::btree_set(0..GRAPHS, 0..=GRAPHS).prop_map(|s| s.into_iter().collect())
}

fn valid(text: &str) -> bool {
    sparql::parse(text).is_ok()
}

fn forms(body: &str) -> [String; 3] {
    [
        format!("SELECT * WHERE {{ {body} }}"),
        format!("ASK {{ {body} }}"),
        format!("CONSTRUCT {{ ?s <https://ex.org/out> ?o }} WHERE {{ {body} }}"),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    /// Graph selection plus pre-evaluated property functions give the same answers as the
    /// reference query (VALUES written by hand) over a default graph merged from the selection.
    #[test]
    fn rewritten_queries_mean_what_the_agent_wrote(p in pattern(false), data in facts(), selected in selection()) {
        prop_assume!(valid(&format!("SELECT * WHERE {{ {} }}", p.render(Calls::Function))));
        let store = store(&data);
        let reference_data = merged(&data, &selected);
        for (agent, reference) in forms(&p.render(Calls::Function)).iter().zip(forms(&p.render(Calls::Values))) {
            let sent = rewritten(agent, &selected);
            prop_assert_eq!(evaluate(&sent, &store), evaluate(&reference, &reference_data), "\nagent: {}\nsent: {}", agent, sent);
        }
    }

    /// Writing a parsed query back out keeps its meaning. spargebra's own printer does not, for a
    /// FILTER in some positions; `to_sparql` exists for that.
    #[test]
    fn printing_a_query_keeps_its_meaning(p in pattern(false), data in facts()) {
        let store = store(&data);
        for text in forms(&p.render(Calls::Values)) {
            prop_assume!(valid(&text));
            let (query, _) = sparql::parse(&text).unwrap_or_else(|e| panic!("{e}"));
            let printed = sparql::to_sparql(query);
            prop_assert_eq!(evaluate(&printed, &store), evaluate(&text, &store), "\nwritten: {}\nprinted: {}", text, printed);
        }
    }

    /// Every local call site is found and replaced; nothing else in the query changes.
    #[test]
    fn substitution_replaces_exactly_the_local_calls(p in pattern(true)) {
        let text = format!("SELECT * WHERE {{ {} }}", p.render(Calls::Function));
        prop_assume!(valid(&text));
        let (query, _) = sparql::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let (local, remote) = p.calls();
        let sites = sparql::sites(&query, &is_function).unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(sites.len(), local);
        for site in &sites {
            let (alone, _) = sparql::parse(&site.query()).unwrap_or_else(|e| panic!("{e}"));
            let analysis = sparql::analyze(&alone);
            prop_assert_eq!(analysis.functions.len(), 1);
            prop_assert!(!analysis.has_dataset);
        }

        let before = sparql::analyze(&query);
        let tables: VecDeque<_> = sites.iter().map(split_rows).collect();
        let replaced = sparql::substitute(query, &is_function, tables).unwrap_or_else(|e| panic!("{e}"));
        let sent = sparql::to_sparql(replaced);
        let (again, _) = sparql::parse(&sent).unwrap_or_else(|e| panic!("{e}\n{sent}"));
        let after = sparql::analyze(&again);

        prop_assert!(sparql::sites(&again, &is_function).unwrap_or_default().is_empty());
        prop_assert_eq!(sent.contains(SPLIT), remote > 0, "calls inside SERVICE stay for the endpoint: {}", sent);
        prop_assert_eq!(&before.services, &after.services);
        prop_assert_eq!(&before.remote_terms, &after.remote_terms);
        let kept: BTreeSet<&String> = before.terms.iter().filter(|t| t.as_str() != SPLIT && t.as_str() != sparql::RDF_NIL).collect();
        let now: BTreeSet<&String> = after.terms.iter().collect();
        prop_assert!(kept.is_subset(&now), "terms lost: {:?}", kept.difference(&now).collect::<Vec<_>>());
    }

    /// A dataset clause survives serialisation, and the analysis sees the same terms and graphs.
    #[test]
    fn a_dataset_clause_changes_nothing_but_the_dataset(p in pattern(true), selected in selection()) {
        let text = format!("SELECT * WHERE {{ {} }}", p.render(Calls::Function));
        prop_assume!(valid(&text));
        let (query, _) = sparql::parse(&text).unwrap_or_else(|e| panic!("{e}"));
        let before = sparql::analyze(&query);
        let default: Vec<String> = selected.iter().map(|g| graph(*g)).collect();
        let sent = sparql::with_dataset(query, &default, &(0..GRAPHS).map(graph).collect::<Vec<_>>())
            .unwrap_or_else(|e| panic!("{e}"));
        let sent = sparql::to_sparql(sent);
        let (again, _) = sparql::parse(&sent).unwrap_or_else(|e| panic!("{e}\n{sent}"));
        let after = sparql::analyze(&again);
        prop_assert!(after.has_dataset);
        prop_assert_eq!(&before.terms, &after.terms);
        prop_assert_eq!(&before.services, &after.services);
        prop_assert_eq!(before.functions.len(), after.functions.len());
        prop_assert!(before.graphs.is_subset(&after.graphs));
    }
}
