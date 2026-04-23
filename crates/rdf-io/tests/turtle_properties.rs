mod common;

use common::quads_isomorphic;
use proptest::prelude::*;
use rdf_io::{parse_turtle, serialize_turtle_to_string};
use rdf_model::{BlankNode, GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject};

fn iri_strategy() -> impl Strategy<Value = NamedNode> {
    "[a-z][a-z0-9]{0,10}".prop_map(|suffix| {
        NamedNode::new(format!("http://example.com/{suffix}")).expect("valid generated IRI")
    })
}

fn blank_node_strategy() -> impl Strategy<Value = BlankNode> {
    "[a-z_][a-z0-9_.-]{0,10}"
        .prop_map(|label| BlankNode::new(label).expect("valid generated blank node"))
}

fn literal_strategy() -> impl Strategy<Value = Literal> {
    let simple = "[A-Za-z0-9 _-]{0,10}".prop_map(Literal::new_simple);
    let lang = ("[A-Za-z0-9 _-]{0,10}", "[a-z]{2}").prop_map(|(lexical, lang)| {
        Literal::new_language_tagged(lexical, lang).expect("valid generated language-tagged")
    });
    let typed = ("[A-Za-z0-9 _-]{0,10}", iri_strategy()).prop_map(|(lexical, datatype)| {
        Literal::new_typed(lexical, datatype).expect("valid generated typed literal")
    });
    prop_oneof![simple, lang, typed]
}

fn subject_strategy() -> impl Strategy<Value = Subject> {
    prop_oneof![
        iri_strategy().prop_map(Subject::from),
        blank_node_strategy().prop_map(Subject::from),
    ]
}

fn object_strategy() -> impl Strategy<Value = Object> {
    prop_oneof![
        iri_strategy().prop_map(Object::from),
        blank_node_strategy().prop_map(Object::from),
        literal_strategy().prop_map(Object::from),
    ]
}

fn quad_strategy() -> impl Strategy<Value = Quad> {
    (subject_strategy(), iri_strategy(), object_strategy()).prop_map(
        |(subject, predicate, object)| {
            Quad::new(subject, Predicate::from(predicate), object, GraphName::DefaultGraph)
        },
    )
}

proptest! {
    #[test]
    fn turtle_round_trip_is_graph_isomorphic(quads in prop::collection::vec(quad_strategy(), 0..20)) {
        let encoded = serialize_turtle_to_string(&quads).expect("serialize turtle quads");
        let decoded = parse_turtle(&encoded).expect("parse serialized turtle");
        prop_assert!(quads_isomorphic(&quads, &decoded));
    }

    #[test]
    fn turtle_parse_is_deterministic(quads in prop::collection::vec(quad_strategy(), 0..20)) {
        let encoded = serialize_turtle_to_string(&quads).expect("serialize turtle quads");
        let left = parse_turtle(&encoded);
        let right = parse_turtle(&encoded);
        prop_assert_eq!(left, right);
    }
}
