use rdf_io::{NQuadsErrorKind, parse_nquads, parse_nquads_reader, serialize_nquads_to_string};
use rdf_model::{GraphName, NamedNode, Subject};
use std::io::Cursor;

#[test]
fn parses_valid_nquads_records() {
    let input = r#"
<http://example.com/s> <http://example.com/p> "o" .
<http://example.com/s2> <http://example.com/p> "o2" <http://example.com/g> .
"#;
    let quads = parse_nquads(input).expect("valid n-quads");
    assert_eq!(quads.len(), 2);
    assert_eq!(quads[0].graph_name(), &GraphName::DefaultGraph);
}

#[test]
fn reports_line_and_column_for_nquads_errors() {
    let input = r#"<http://example.com/s> <http://example.com/p> "o" <http://example.com/g>"#;
    let err = parse_nquads(input).expect_err("missing dot must fail");
    assert_eq!(err.kind, NQuadsErrorKind::ExpectedDot);
    assert_eq!(err.line, 1);
    assert!(err.column > 1);
}

#[test]
fn reader_parser_parses_streaming_nquads() {
    let input = Cursor::new(
        "<http://example.com/s> <http://example.com/p> \"o\" .\n\
         <http://example.com/s2> <http://example.com/p> \"o2\" <http://example.com/g> .\n",
    );
    let parsed =
        parse_nquads_reader(input).collect::<Result<Vec<_>, _>>().expect("parse from reader");
    assert_eq!(parsed.len(), 2);
}

#[test]
fn nquads_serialize_round_trip_is_semantic() {
    let input = r#"<http://example.com/s> <http://example.com/p> "o" <http://example.com/g> ."#;
    let parsed = parse_nquads(input).expect("parse input");
    let encoded = serialize_nquads_to_string(&parsed).expect("serialize");
    let decoded = parse_nquads(&encoded).expect("parse encoded");
    assert_eq!(parsed, decoded);
}

#[test]
fn graph_term_is_parsed() {
    let quads =
        parse_nquads("_:a <http://example.com/p> <http://example.com/o> _:g .\n").expect("parse");
    assert_eq!(quads.len(), 1);
    assert!(matches!(quads[0].subject(), Subject::BlankNode(_)));
    assert!(matches!(quads[0].graph_name(), GraphName::BlankNode(_)));
    let _iri = NamedNode::new("http://example.com/p").expect("iri");
}
