//! The SPARQL handling and rendering, without a server.
use rdf_mcp::client::{check_dataset, check_loopback};
use rdf_mcp::render::{self, GraphKind, Prefixes};
use rdf_mcp::sparql::{self, Form};
use spargebra::term::{GroundTerm, Literal, NamedNode};
use std::collections::VecDeque;

fn parse(q: &str) -> spargebra::Query {
    sparql::parse(q).map(|(q, _)| q).unwrap_or_else(|e| panic!("{e}"))
}
fn is_function(iri: &str) -> bool {
    iri.starts_with("urn:rdftool:search:") || iri == "urn:rdftool:property:split"
}

#[test]
fn updates_are_refused_with_a_reason() {
    let error = sparql::parse("DELETE WHERE { ?s ?p ?o }").err().unwrap_or_default();
    assert!(error.contains("read-only"), "{error}");
    let error = sparql::parse("SELEC * { ?s ?p ?o }").err().unwrap_or_default();
    assert!(!error.contains("read-only"), "{error}");
    assert!(matches!(sparql::parse("ASK { ?s ?p ?o }"), Ok((_, Form::Ask))));
}

#[test]
fn analysis_separates_local_remote_and_function_terms() {
    let q = parse(
        "PREFIX ex: <https://ex.org/> SELECT * WHERE { \
           ?c a ex:Claim ; ex:hasPolicy ?p . \
           GRAPH <urn:g:data> { ?p ex:number ?n } \
           (?s ?pr ?o ?score) <urn:rdftool:search:hybrid> (\"risk\" ?text 5) \
           SERVICE <http://ontop:8080/sparql> { ?p ex:remoteOnly ?x } \
           FILTER(?n != ex:none) }",
    );
    let a = sparql::analyze(&q);
    assert!(a.terms.contains("https://ex.org/Claim"));
    assert!(a.terms.contains("https://ex.org/hasPolicy"));
    assert!(a.terms.contains("https://ex.org/none"), "terms in FILTER count");
    assert!(a.remote_terms.contains("https://ex.org/remoteOnly"));
    assert!(!a.terms.contains("https://ex.org/remoteOnly"));
    assert!(a.services.contains("http://ontop:8080/sparql"));
    assert!(a.graphs.contains("urn:g:data"));
    assert!(!a.has_dataset);
    let call = &a.functions[0];
    assert_eq!(call.iri, "urn:rdftool:search:hybrid");
    assert_eq!((call.inputs, call.outputs), (Some(3), Some(4)));
    assert_eq!(call.variable_inputs, vec!["?text".to_owned()]);
}

#[test]
fn a_dataset_clause_is_added_and_survives_reparsing() {
    let q = parse("SELECT ?x WHERE { ?x a <https://ex.org/C> . GRAPH ?g { ?x ?p ?o } }");
    let graphs = vec!["urn:g:a".to_owned(), "urn:g:b".to_owned()];
    let text =
        sparql::with_dataset(q, &graphs, &graphs).unwrap_or_else(|e| panic!("{e}")).to_string();
    assert!(text.contains("FROM <urn:g:a>") && text.contains("FROM NAMED <urn:g:b>"), "{text}");
    let again = sparql::analyze(&parse(&text));
    assert!(again.has_dataset);
    assert!(sparql::with_dataset(parse("ASK {}"), &["not an iri".to_owned()], &[]).is_err());
}

#[test]
fn property_function_calls_are_replaced_by_their_rows() {
    let q = parse(
        "SELECT ?s ?type WHERE { (?s ?p ?o ?score) <urn:rdftool:search:hybrid> (\"risk-notes\" \"theft\" 2) . ?s a ?type }",
    );
    let sites = sparql::sites(&q, &is_function).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sites.len(), 1);
    let site_query = sites[0].query();
    assert!(
        site_query.contains("<urn:rdftool:search:hybrid>") && site_query.contains("\"theft\""),
        "{site_query}"
    );
    assert!(!site_query.contains("FROM"));

    let iri =
        |s: &str| Some(GroundTerm::NamedNode(NamedNode::new(s).unwrap_or_else(|e| panic!("{e}"))));
    let lit = |s: &str| Some(GroundTerm::Literal(Literal::new_simple_literal(s)));
    let table =
        vec![vec![iri("https://ex.org/a"), iri("https://ex.org/note"), lit("stolen"), lit("0.9")]];
    let replaced = sparql::substitute(q, &is_function, VecDeque::from([table]))
        .unwrap_or_else(|e| panic!("{e}"));
    let text = sparql::with_dataset(replaced, &["urn:g:a".to_owned()], &["urn:g:a".to_owned()])
        .unwrap_or_else(|e| panic!("{e}"))
        .to_string();
    assert!(!text.contains("urn:rdftool:search"), "{text}");
    assert!(!text.contains("rdf-syntax-ns#first"), "list helper triples removed: {text}");
    assert!(
        text.contains("VALUES")
            && text.contains("https://ex.org/a")
            && text.contains("FROM <urn:g:a>"),
        "{text}"
    );
    let again = sparql::analyze(&parse(&text));
    assert!(again.terms.contains("https://ex.org/a"));
}

#[test]
fn function_inputs_must_be_constants() {
    let q = parse("SELECT * WHERE { (?s ?p ?o ?score) <urn:rdftool:search:hybrid> (\"i\" ?t 2) }");
    assert!(sparql::sites(&q, &is_function).is_err());
}

#[test]
fn terms_render_as_sparql() {
    let t = |v: serde_json::Value| render::term(&v).as_str().map(str::to_owned).unwrap_or_default();
    assert_eq!(
        t(serde_json::json!({"type":"uri","value":"https://ex.org/a"})),
        "<https://ex.org/a>"
    );
    assert_eq!(t(serde_json::json!({"type":"literal","value":"hi \"x\""})), "\"hi \\\"x\\\"\"");
    assert_eq!(
        t(serde_json::json!({"type":"literal","value":"chat","xml:lang":"fr"})),
        "\"chat\"@fr"
    );
    assert_eq!(
        t(
            serde_json::json!({"type":"literal","value":"42","datatype":"http://www.w3.org/2001/XMLSchema#integer"})
        ),
        "42"
    );
    assert_eq!(
        t(
            serde_json::json!({"type":"literal","value":"2019-06-02","datatype":"http://www.w3.org/2001/XMLSchema#date"})
        ),
        "\"2019-06-02\"^^<http://www.w3.org/2001/XMLSchema#date>"
    );
}

#[test]
fn prefixes_suggestions_and_graph_kinds() {
    let p = Prefixes::build([
        "http://data.world/schema/insurance/Claim",
        "http://www.w3.org/2000/01/rdf-schema#label",
    ]);
    assert_eq!(p.curie("http://data.world/schema/insurance/Claim"), "insurance:Claim");
    assert_eq!(p.curie("http://www.w3.org/2000/01/rdf-schema#label"), "rdfs:label");
    assert_eq!(p.expand("insurance:Claim"), "http://data.world/schema/insurance/Claim");
    assert_eq!(p.expand("<urn:x>"), "urn:x");
    assert!(p.block().contains("PREFIX insurance: <http://data.world/schema/insurance/>"));

    let known = vec![
        "http://data.world/schema/insurance/Claim".to_owned(),
        "http://data.world/schema/insurance/claimNumber".to_owned(),
    ];
    assert_eq!(
        render::suggest("http://data.world/schema/insurance/Clam", &known),
        vec![known[0].clone()]
    );
    assert!(render::suggest("http://data.world/schema/insurance/Zebra", &known).is_empty());

    assert_eq!(
        GraphKind::of("urn:rdftool:derived:inference:acme"),
        GraphKind::Inferred("acme".into())
    );
    assert_eq!(GraphKind::of("urn:rdftool:derived:inference:acme:metadata"), GraphKind::Metadata);
    assert_eq!(
        GraphKind::of("urn:rdftool:derived:shacl:c"),
        GraphKind::ValidationReport("c".into())
    );
    assert_eq!(GraphKind::of("urn:example:data"), GraphKind::Asserted);
}

#[test]
fn inputs_that_reach_urls_or_queries_are_checked() {
    assert!(render::iri("<https://ex.org/a>").is_ok());
    assert!(render::iri("https://ex.org/a> } DROP ALL #").is_err());
    assert!(render::iri("plain words").is_err());
    assert!(check_dataset("claims").is_ok());
    assert!(check_dataset("../admin").is_err());
    assert!(check_loopback("http://127.0.0.1:7878").is_ok());
    assert!(check_loopback("http://localhost:7878").is_ok());
    assert!(check_loopback("http://[::1]:7878").is_ok());
    assert!(check_loopback("http://10.0.0.5:7878").is_err());
    assert!(check_loopback("http://example.org").is_err());
}

/// Results of a query over a small dataset, as a set of canonical rows.
fn rows(text: &str, data: &oxrdf::Dataset) -> std::collections::BTreeSet<String> {
    let query =
        spargebra::SparqlParser::new().parse_query(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let evaluator = spareval::QueryEvaluator::new();
    match evaluator.prepare(&query).execute(data).unwrap_or_else(|e| panic!("{e}")) {
        spareval::QueryResults::Solutions(s) => s
            .map(|s| {
                let s = s.unwrap_or_else(|e| panic!("{e}"));
                let mut cells: Vec<String> = s.iter().map(|(v, t)| format!("{v}={t}")).collect();
                cells.sort();
                cells.join(" ")
            })
            .collect(),
        _ => panic!("SELECT expected"),
    }
}

/// Two shapes spargebra 0.4.8 prints with a different meaning (found by the property tests).
#[test]
fn printing_keeps_a_filter_in_its_own_group() {
    let ex = |s: &str| {
        oxrdf::NamedNode::new(format!("https://ex.org/{s}")).unwrap_or_else(|e| panic!("{e}"))
    };
    let data: oxrdf::Dataset = [
        oxrdf::Quad::new(ex("i0"), ex("p1"), ex("i4"), oxrdf::GraphName::DefaultGraph),
        oxrdf::Quad::new(ex("i1"), ex("p2"), ex("i3"), oxrdf::GraphName::DefaultGraph),
    ]
    .into_iter()
    .collect();
    for text in [
        // FILTER on the left of an inner OPTIONAL, inside an outer OPTIONAL.
        "SELECT * WHERE { { ?s <https://ex.org/p1> ?v } OPTIONAL { { { <https://ex.org/i0> <https://ex.org/p1> ?o } \
         FILTER(!BOUND(?v) || ?v != <https://ex.org/i4>) } OPTIONAL { ?s <https://ex.org/p1> <https://ex.org/i2> } } }",
        // A filtered group as the whole right side of an OPTIONAL.
        "SELECT * WHERE { { ?o <https://ex.org/p2> ?x } OPTIONAL { { VALUES ?s { \"v0\" } \
         FILTER(!BOUND(?o) || ?o != <https://ex.org/i1>) } } }",
    ] {
        let (query, _) = sparql::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let plain = query.to_string();
        let safe = sparql::to_sparql(query);
        assert_ne!(
            rows(&plain, &data),
            rows(text, &data),
            "spargebra's printer fixed? then to_sparql may be simplified"
        );
        assert_eq!(rows(&safe, &data), rows(text, &data), "\n{safe}");
    }
}
