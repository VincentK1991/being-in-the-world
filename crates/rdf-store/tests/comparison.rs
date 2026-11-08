use rdf_store::comparison::{Literals, by_value, solutions_for};
use rdf_store::oxigraph::model::{Literal, NamedNode, Term};

fn typed(value: &str, datatype: &str) -> Term {
    Literal::new_typed_literal(
        value,
        NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{datatype}")),
    )
    .into()
}

#[test]
fn numbers_and_booleans_compare_by_value() {
    for (a, b, datatype) in [
        ("01", "1", "integer"),
        ("+1", "1", "int"),
        ("1.0", "1", "decimal"),
        ("-0.0", "0", "decimal"),
        ("0010.500", "10.5", "decimal"),
        ("1.0e0", "1", "double"),
        ("3.0E4", "30000", "double"),
        ("1", "true", "boolean"),
    ] {
        assert_eq!(
            by_value(typed(a, datatype)),
            by_value(typed(b, datatype)),
            "{a} = {b} as {datatype}"
        );
    }
    assert_ne!(by_value(typed("1", "integer")), by_value(typed("2", "integer")));
    // Ill-formed lexical forms are left as written, never coerced.
    assert_eq!(by_value(typed("abc", "integer")), typed("abc", "integer"));
    assert_eq!(by_value(typed("1.", "decimal")), by_value(typed("1", "decimal")));
    assert_eq!(by_value(typed(".", "decimal")), typed(".", "decimal"));
}

#[test]
fn exact_comparison_keeps_lexical_forms_apart() {
    let row = |v: &str| vec![("x".to_owned(), typed(v, "integer"))];
    let query = "SELECT ?x WHERE { ?s ?p ?x }";
    let exact = |v| {
        solutions_for(vec!["x".into()], vec![row(v)], query, Literals::Exact).unwrap_or_default()
    };
    let value = |v| {
        solutions_for(vec!["x".into()], vec![row(v)], query, Literals::ByValue).unwrap_or_default()
    };
    assert_ne!(exact("01"), exact("1"));
    assert_eq!(value("01"), value("1"));
}

#[test]
fn reduced_compares_as_a_set() {
    let row = vec![("x".to_owned(), typed("1", "integer"))];
    let compare = |query: &str, rows: Vec<Vec<(String, Term)>>| {
        solutions_for(vec!["x".into()], rows, query, Literals::Exact).unwrap_or_default()
    };
    let reduced = "SELECT REDUCED ?x WHERE { ?s ?p ?x }";
    assert_eq!(
        compare(reduced, vec![row.clone(), row.clone()]),
        compare(reduced, vec![row.clone()])
    );
    let plain = "SELECT ?x WHERE { ?s ?p ?x }";
    assert_ne!(compare(plain, vec![row.clone(), row.clone()]), compare(plain, vec![row]));
}
