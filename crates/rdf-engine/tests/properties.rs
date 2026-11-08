use anyhow::Result;
use rdf_engine::Database;
use rdf_plugin_api::{
    Manifest, Plugin, PropertyDescriptor, PropertyFunction, RdfValue, ReadContext, RowSink,
    SDK_VERSION,
};
use rdf_plugin_custom_properties::CustomProperties;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn database() -> Result<Database> {
    let mut db = Database::memory()?;
    db.register(CustomProperties::examples())?;
    Ok(db)
}
fn rows(db: &Database, query: &str) -> Result<Vec<Value>> {
    Ok(db.query_json(query)?["results"]["bindings"].as_array().expect("bindings").clone())
}
#[test]
fn properties_generate_rows_and_join_real_data() -> Result<()> {
    let db = database()?;
    let result = rows(
        &db,
        "SELECT ?word ?position WHERE { (?word ?position) <urn:rdftool:property:tokens> 'red blue red' } ORDER BY ?position",
    )?;
    assert_eq!(result.len(), 3);
    assert_eq!(result[0]["word"]["value"], "red");
    assert_eq!(result[2]["position"]["value"], "2");
    assert_eq!(result[2]["position"]["datatype"], "http://www.w3.org/2001/XMLSchema#integer");
    db.update("INSERT DATA { <urn:red> <urn:label> 'red' . <urn:blue> <urn:label> 'blue' }")?;
    let result = rows(
        &db,
        "PREFIX p: <urn:rdftool:property:> SELECT ?s WHERE { ?s <urn:label> ?word . ?word p:split ('red,red,missing' ',') } ORDER BY ?s",
    )?;
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["s"]["value"], "urn:red");
    assert_eq!(db.overview()?.quads, 2); // Virtual relation is never materialized.
    Ok(())
}
#[test]
fn sparql_algebra_handles_optional_union_filters_aggregates_and_subqueries() -> Result<()> {
    let db = database()?;
    let cases = [
        (
            "SELECT ?word WHERE { ?word <urn:rdftool:property:split> ('a,b,a' ',') FILTER(?word = 'a') }",
            2,
        ),
        ("SELECT DISTINCT ?word WHERE { ?word <urn:rdftool:property:split> ('a,b,a' ',') }", 2),
        (
            "SELECT ?word WHERE { { ?word <urn:rdftool:property:split> ('a' ',') } UNION { ?word <urn:rdftool:property:split> ('b' ',') } }",
            2,
        ),
        (
            "SELECT ?word WHERE { VALUES ?v { 1 } OPTIONAL { (?word ?position) <urn:rdftool:property:tokens> '' } }",
            1,
        ),
        (
            "SELECT ?word WHERE { { SELECT ?word WHERE { ?word <urn:rdftool:property:split> ('a,b' ',') } LIMIT 1 } }",
            1,
        ),
        (
            "SELECT ?v WHERE { VALUES ?v { 1 2 } FILTER EXISTS { ?word <urn:rdftool:property:split> ('a' ',') } }",
            2,
        ),
        (
            "SELECT ?v WHERE { VALUES ?v { 1 } FILTER NOT EXISTS { (?word ?position) <urn:rdftool:property:tokens> '' } }",
            1,
        ),
    ];
    for (query, expected) in cases {
        assert_eq!(rows(&db, query)?.len(), expected, "{query}");
    }
    assert_eq!(
        rows(
            &db,
            "SELECT (COUNT(*) AS ?n) WHERE { ?word <urn:rdftool:property:split> ('a,a,b' ',') }"
        )?[0]["n"]["value"],
        "3"
    );
    assert_eq!(
        db.query_json("ASK { (<urn:missing> 0) <urn:rdftool:property:tokens> 'a' }")?["boolean"],
        false
    );
    assert_eq!(
        db.query_json("ASK { ('a' 0) <urn:rdftool:property:tokens> 'a' }")?["boolean"],
        true
    );
    assert_eq!(
        rows(&db, "SELECT ?x WHERE { (?x ?x) <urn:rdftool:property:tokens> '0' }")?.len(),
        0
    );
    Ok(())
}
#[test]
fn rejects_unsupported_and_invalid_calls_without_affecting_ordinary_rdf() -> Result<()> {
    let db = database()?;
    for query in [
        "SELECT * WHERE { ?word <urn:rdftool:property:split> ('a' '') }",
        "SELECT * WHERE { ?word <urn:rdftool:property:split> (1 ',') }",
        "SELECT * WHERE { ?word <urn:rdftool:property:split> ('a'@en ',') }",
        "SELECT * WHERE { ?word <urn:rdftool:property:split> ('a') }",
        "SELECT * WHERE { VALUES ?text { 'a' } ?word <urn:rdftool:property:split> (?text ',') }",
        "SELECT * WHERE { ?word <urn:rdftool:property:missing> 'a' }",
        "SELECT * WHERE { GRAPH <urn:g> { ?word <urn:rdftool:property:split> ('a' ',') } }",
        "SELECT * FROM <urn:g> WHERE { ?word <urn:rdftool:property:split> ('a' ',') }",
        "SELECT * WHERE { ?s <urn:rdftool:property:split>* ?o }",
        "SELECT (<urn:rdftool:property:split>('a', ',') AS ?x) WHERE {}",
        "SELECT * WHERE { SERVICE SILENT <urn:remote> { ?word <urn:rdftool:property:split> ('a' ',') } }",
    ] {
        assert!(db.query(query).is_err(), "must reject: {query}");
    }
    assert!(db.update("INSERT { <urn:s> <urn:p> ?word } WHERE { ?word <urn:rdftool:property:split> ('a' ',') }").is_err());
    assert_eq!(db.overview()?.quads, 0);
    assert_eq!(
        rows(&db, "SELECT ?v WHERE { VALUES ?v { 'urn:rdftool:property:split' } }")?.len(),
        1
    );
    assert_eq!(rows(&db, "SELECT * WHERE { ?s <urn:ordinary:unknown> ?o }")?.len(), 0);
    Ok(())
}

struct Relation {
    iri: &'static str,
    calls: Arc<AtomicUsize>,
    invalid: bool,
}
impl PropertyFunction for Relation {
    fn descriptor(&self) -> PropertyDescriptor {
        PropertyDescriptor {
            iri: self.iri.into(),
            description: "test relation".into(),
            input_arity: 0,
            output_arity: 2,
        }
    }
    fn evaluate(
        &self,
        context: &dyn ReadContext,
        _: &[RdfValue],
        out: &mut dyn RowSink,
    ) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid {
            return out.emit(vec![RdfValue::Iri("invalid IRI".into()), RdfValue::integer(1)]);
        }
        let count = context.query_json("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }")?["results"]["bindings"][0]["n"]["value"].as_str().expect("count").parse::<usize>()?;
        out.emit(vec![RdfValue::Iri("urn:hit".into()), RdfValue::integer(count)])
    }
}
#[test]
fn arbitrary_properties_use_selected_dataset_and_registration_is_atomic() -> Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut a = database()?;
    let mut b = database()?;
    // A separate plugin can define an arbitrary property IRI without engine changes.
    struct Other(Relation);
    impl Plugin for Other {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "other".into(),
                version: "1".into(),
                sdk_version: SDK_VERSION,
                operations: vec![],
                properties: vec![],
            }
        }
        fn invoke(&self, _: &dyn ReadContext, _: &str, _: Value) -> Result<Value> {
            Ok(json!(null))
        }
        fn property_functions(&self) -> Vec<Arc<dyn PropertyFunction>> {
            vec![Arc::new(Relation {
                iri: self.0.iri,
                calls: self.0.calls.clone(),
                invalid: self.0.invalid,
            })]
        }
    }
    for db in [&mut a, &mut b] {
        db.register(Other(Relation {
            iri: "https://example.org/relation",
            calls: calls.clone(),
            invalid: false,
        }))?;
    }
    a.update("INSERT DATA { <urn:s> <urn:p> 1 }")?;
    let query = "SELECT ?hit ?n WHERE { (?hit ?n) <https://example.org/relation> () }";
    assert_eq!(rows(&a, query)?[0]["n"]["value"], "1");
    assert_eq!(rows(&b, query)?[0]["n"]["value"], "0");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(a.query("SELECT * WHERE { (?hit ?n) <https://example.org/relation> () . ?bad <urn:rdftool:property:missing> 'x' }").is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2); // Complete validation precedes evaluation.
    let manifest = a.plugins().into_iter().find(|p| p.id == "other").expect("registered");
    assert_eq!(manifest.properties[0].iri, "https://example.org/relation");
    Ok(())
}
#[test]
fn bounded_expansion_and_invalid_plugin_values_fail_queries() -> Result<()> {
    let db = database()?;
    let query = format!(
        "SELECT * WHERE {{ ?word <urn:rdftool:property:split> ('{}' ',') }} LIMIT 1",
        "a,".repeat(10_001)
    );
    assert!(db.query(&query).is_err());
    let mut invalid = Database::memory()?;
    invalid.register(CustomProperties::new().with_property(Relation {
        iri: "urn:test:invalid",
        calls: Arc::default(),
        invalid: true,
    }))?;
    assert!(invalid.query("SELECT * WHERE { (?a ?b) <urn:test:invalid> () }").is_err());
    let mut duplicate = Database::memory()?;
    assert!(
        duplicate
            .register(
                CustomProperties::new()
                    .with_property(Relation {
                        iri: "urn:duplicate",
                        calls: Arc::default(),
                        invalid: false
                    })
                    .with_property(Relation {
                        iri: "urn:duplicate",
                        calls: Arc::default(),
                        invalid: false
                    })
            )
            .is_err()
    );
    assert!(duplicate.plugins().is_empty());
    Ok(())
}
#[test]
fn property_calls_survive_blank_nodes_rewritten_to_variables() -> Result<()> {
    // A federation engine forwards a sub-pattern with its list cells turned into variables (?g_0 ...).
    let db = database()?;
    let result = rows(
        &db,
        r#"PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
        SELECT ?word ?position WHERE {
          ?g_0 rdf:first ?word ; rdf:rest ?g_1 . ?g_1 rdf:first ?position ; rdf:rest rdf:nil .
          ?g_0 <urn:rdftool:property:tokens> 'red blue red' .
        } ORDER BY ?position"#,
    )?;
    assert_eq!(result.len(), 3);
    assert_eq!(result[1]["word"]["value"], "blue");
    let split = rows(
        &db,
        r#"PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
        SELECT ?part WHERE {
          ?in rdf:first 'a,b' ; rdf:rest ?in2 . ?in2 rdf:first ',' ; rdf:rest rdf:nil .
          ?part <urn:rdftool:property:split> ?in .
        } ORDER BY ?part"#,
    )?;
    assert_eq!(
        split.iter().map(|r| r["part"]["value"].as_str().unwrap_or("")).collect::<Vec<_>>(),
        ["a", "b"]
    );
    // Stored RDF lists are still never traversed: these patterns match data, not a property call.
    db.update("INSERT DATA { <urn:l> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> 'x' }")?;
    let stored = rows(
        &db,
        "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> SELECT ?v WHERE { ?cell rdf:first ?v }",
    )?;
    assert_eq!(stored.len(), 1);
    Ok(())
}

#[test]
fn catalog_lists_the_plugins_a_dataset_has() -> Result<()> {
    let mut db = Database::memory()?;
    db.register(CustomProperties::examples())?;
    db.register(rdf_plugin_catalog::PluginCatalog::new(db.catalog()))?;
    let found = rows(
        &db,
        "SELECT ?plugin ?version WHERE { (?plugin ?version) <urn:rdftool:plugins:available> () } ORDER BY ?plugin",
    )?;
    let names: Vec<&str> =
        found.iter().map(|r| r["plugin"]["value"].as_str().expect("plugin")).collect();
    assert_eq!(names, ["custom-properties", "plugin-catalog"]);
    // Joined with ordinary patterns and filters, like any property function.
    let provided = rows(
        &db,
        "SELECT ?name WHERE { (?plugin ?kind ?name) <urn:rdftool:plugins:provides> () \
         FILTER(?plugin = 'plugin-catalog' && ?kind = 'property-function') } ORDER BY ?name",
    )?;
    assert_eq!(provided.len(), 2);
    // A dataset without the catalog plugin answers with no rows rather than an error.
    let mut bare = Database::memory()?;
    bare.register(CustomProperties::examples())?;
    assert!(
        rows(&bare, "SELECT * WHERE { (?p ?v) <urn:rdftool:plugins:available> () }")?.is_empty()
    );
    // Inputs are rejected: the object must be ().
    assert!(db.query("SELECT * WHERE { (?p ?v) <urn:rdftool:plugins:available> (1) }").is_err());
    Ok(())
}
