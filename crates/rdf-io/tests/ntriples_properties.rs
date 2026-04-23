use proptest::prelude::*;
use rdf_io::{parse_ntriples, serialize_ntriples_to_string};
use rdf_model::{BlankNode, Literal, NamedNode, Object, Predicate, Subject, Triple};

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

fn triple_strategy() -> impl Strategy<Value = Triple> {
    (subject_strategy(), iri_strategy(), object_strategy()).prop_map(
        |(subject, predicate, object)| Triple::new(subject, Predicate::from(predicate), object),
    )
}

proptest! {
    #[test]
    fn parse_serialize_round_trip_is_semantically_stable(triples in prop::collection::vec(triple_strategy(), 0..20)) {
        let encoded = serialize_ntriples_to_string(&triples).expect("serialize triples");
        let decoded = parse_ntriples(&encoded).expect("parse serialized triples");
        prop_assert_eq!(decoded, triples);
    }

    #[test]
    fn parse_is_deterministic_for_same_input(triples in prop::collection::vec(triple_strategy(), 0..20)) {
        let encoded = serialize_ntriples_to_string(&triples).expect("serialize triples");
        let left = parse_ntriples(&encoded);
        let right = parse_ntriples(&encoded);
        prop_assert_eq!(left, right);
    }
}
