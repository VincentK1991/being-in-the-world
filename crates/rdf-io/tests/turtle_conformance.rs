mod common;

use common::quads_isomorphic;
use rdf_io::{TurtleErrorKind, parse_turtle, parse_turtle_reader, serialize_turtle_to_string};
use rdf_model::GraphName;
use std::io::Cursor;

#[test]
fn parses_valid_turtle_with_prefixes() {
    let input = r#"
@prefix ex: <http://example.com/> .
ex:s ex:p "o" .
ex:s ex:p2 "hello"@en .
"#;
    let quads = parse_turtle(input).expect("valid turtle");
    assert_eq!(quads.len(), 2);
    assert!(quads.iter().all(|quad| quad.graph_name() == &GraphName::DefaultGraph));
}

#[test]
fn turtle_reports_unknown_prefix() {
    let input = "ex:s ex:p \"o\" .";
    let error = parse_turtle(input).expect_err("missing prefix must fail");
    assert_eq!(error.kind, TurtleErrorKind::UnknownPrefix);
}

#[test]
fn reader_parser_parses_turtle_stream() {
    let input = Cursor::new(
        "@prefix ex: <http://example.com/> .\n\
         ex:s ex:p \"o\" .\n",
    );
    let quads = parse_turtle_reader(input).expect("parse turtle reader");
    assert_eq!(quads.len(), 1);
}

#[test]
fn turtle_serialize_round_trip_is_isomorphic() {
    let input = r#"
@prefix ex: <http://example.com/> .
_:a ex:p ex:o .
"#;
    let parsed = parse_turtle(input).expect("parse turtle");
    let encoded = serialize_turtle_to_string(&parsed).expect("serialize turtle");
    let decoded = parse_turtle(&encoded).expect("parse serialized turtle");
    assert!(quads_isomorphic(&parsed, &decoded));
}
