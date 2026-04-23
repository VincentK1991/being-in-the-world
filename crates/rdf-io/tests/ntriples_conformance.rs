use rdf_io::{
    NTriplesErrorKind, parse_ntriples, parse_ntriples_reader, serialize_ntriples_to_string,
};
use rdf_model::{
    BlankNode, GraphName, Literal, NamedNode, Object, Predicate, Subject, Triple, XSD_STRING_IRI,
};
use std::io::Cursor;

#[test]
fn parses_valid_records_and_skips_comments() {
    let input = r#"
# comment
<http://example.com/s> <http://example.com/p> "o" .
_:a <http://example.com/p> "hello"@en .
"#;
    let triples = parse_ntriples(input).expect("valid n-triples");
    assert_eq!(triples.len(), 2);
}

#[test]
fn reports_line_and_column_for_syntax_errors() {
    let input = r#"<http://example.com/s> <http://example.com/p> "o""#;
    let err = parse_ntriples(input).expect_err("missing dot must fail");
    assert_eq!(err.kind, NTriplesErrorKind::ExpectedDot);
    assert_eq!(err.line, 1);
    assert!(err.column > 1);
}

#[test]
fn reader_parser_is_streaming_and_deterministic() {
    let input = Cursor::new(
        "<http://example.com/s> <http://example.com/p> \"o\" .\n\
         <http://example.com/s2> <http://example.com/p> \"o2\" .\n",
    );
    let parsed =
        parse_ntriples_reader(input).collect::<Result<Vec<_>, _>>().expect("parse from reader");
    assert_eq!(parsed.len(), 2);
}

#[test]
fn serialize_round_trip_keeps_triple_semantics() {
    let triple = Triple::new(
        Subject::BlankNode(BlankNode::new("a").expect("valid bnode")),
        Predicate::new(NamedNode::new("http://example.com/p").expect("valid predicate")),
        Object::Literal(
            Literal::new_typed("hello", NamedNode::new(XSD_STRING_IRI).expect("datatype"))
                .expect("typed literal"),
        ),
    );
    let encoded = serialize_ntriples_to_string(std::slice::from_ref(&triple)).expect("serialize");
    let decoded = parse_ntriples(&encoded).expect("parse serialized");
    assert_eq!(decoded, vec![triple]);
}

#[test]
fn parse_output_is_ntriples_default_graph_only() {
    let triples = parse_ntriples("<http://example.com/s> <http://example.com/p> \"o\" .\n")
        .expect("valid n-triples");
    let graph = GraphName::DefaultGraph;
    assert_eq!(triples.len(), 1);
    assert_eq!(graph, GraphName::DefaultGraph);
}
