mod common;

use common::quads_isomorphic;
use rdf_io::{TrigErrorKind, parse_trig, parse_trig_reader, serialize_trig_to_string};
use rdf_model::GraphName;
use std::io::Cursor;

#[test]
fn parses_valid_trig_graph_blocks() {
    let input = r#"
@prefix ex: <http://example.com/> .
ex:g {
  ex:s ex:p "o" .
}
"#;
    let quads = parse_trig(input).expect("valid trig");
    assert_eq!(quads.len(), 1);
    assert!(matches!(quads[0].graph_name(), GraphName::NamedNode(_)));
}

#[test]
fn trig_reports_syntax_errors() {
    let input = r#"
@prefix ex: <http://example.com/> .
ex:g {
  ex:s ex:p "o" 
}
"#;
    let error = parse_trig(input).expect_err("missing dot must fail");
    assert_eq!(error.kind, TrigErrorKind::UnexpectedToken);
}

#[test]
fn reader_parser_parses_trig_stream() {
    let input = Cursor::new("_:g { _:s <http://example.com/p> \"o\" . }\n");
    let quads = parse_trig_reader(input).expect("parse trig reader");
    assert_eq!(quads.len(), 1);
}

#[test]
fn trig_serialize_round_trip_is_isomorphic() {
    let input = r#"
<http://example.com/g> {
  _:a <http://example.com/p> _:b .
}
"#;
    let parsed = parse_trig(input).expect("parse trig");
    let encoded = serialize_trig_to_string(&parsed).expect("serialize trig");
    let decoded = parse_trig(&encoded).expect("parse serialized trig");
    assert!(quads_isomorphic(&parsed, &decoded));
}
