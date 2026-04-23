use rdf_model::{GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject};
use rdf_store::{MemQuadStorage, QuadPattern};

fn nn(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.com/{local}")).expect("valid IRI")
}

fn quad_default(s: &str, p: &str, o: &str) -> Quad {
    Quad::new(
        Subject::NamedNode(nn(s)),
        Predicate::new(nn(p)),
        Object::Literal(Literal::new_simple(o)),
        GraphName::DefaultGraph,
    )
}

fn quad_named(s: &str, p: &str, o: &str, g: &str) -> Quad {
    Quad::new(
        Subject::NamedNode(nn(s)),
        Predicate::new(nn(p)),
        Object::Literal(Literal::new_simple(o)),
        GraphName::NamedNode(nn(g)),
    )
}

#[test]
fn insert_query_consistency_and_idempotence() {
    let mut store = MemQuadStorage::new();
    let quad = quad_default("s", "p", "o");

    assert!(store.insert(quad.clone()).expect("insert succeeds"));
    assert_eq!(store.len(), 1);

    let pattern = QuadPattern {
        subject: Some(quad.subject().clone()),
        predicate: Some(quad.predicate().clone()),
        object: None,
        graph_name: Some(GraphName::DefaultGraph),
    };
    let matches = store.quads_matching(&pattern);
    assert_eq!(matches, vec![quad.clone()]);

    assert!(!store.insert(quad).expect("duplicate insert reports false"));
    assert_eq!(store.len(), 1);
}

#[test]
fn remove_only_removes_target_quad() {
    let mut store = MemQuadStorage::new();
    let keep = quad_default("s1", "p1", "o1");
    let drop = quad_default("s2", "p2", "o2");

    assert!(store.insert(keep.clone()).expect("insert keep"));
    assert!(store.insert(drop.clone()).expect("insert drop"));

    assert!(store.remove(&drop).expect("remove existing quad"));
    assert!(!store.remove(&drop).expect("remove missing quad returns false"));

    let remaining = store.all_quads();
    assert_eq!(remaining, vec![keep]);
}

#[test]
fn graph_scoped_matching_filters_default_vs_named_graph() {
    let mut store = MemQuadStorage::new();
    let in_default = quad_default("s", "p", "o-default");
    let in_named = quad_named("s", "p", "o-named", "g");
    assert!(store.insert(in_default.clone()).expect("insert default"));
    assert!(store.insert(in_named.clone()).expect("insert named"));

    let named_only = QuadPattern {
        subject: Some(Subject::NamedNode(nn("s"))),
        predicate: Some(Predicate::new(nn("p"))),
        object: None,
        graph_name: Some(GraphName::NamedNode(nn("g"))),
    };
    assert_eq!(store.quads_matching(&named_only), vec![in_named]);

    let default_only = QuadPattern {
        subject: Some(Subject::NamedNode(nn("s"))),
        predicate: Some(Predicate::new(nn("p"))),
        object: None,
        graph_name: Some(GraphName::DefaultGraph),
    };
    assert_eq!(store.quads_matching(&default_only), vec![in_default]);
}
