use rdf_store::{
    engine::Dataset,
    oxigraph::{io::RdfFormat, sparql::QueryResults},
};

#[test]
fn joined_query_update_and_reopen() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    {
        let dataset = Dataset::open(root.path(), "../logical-name", true)?;
        dataset.ingest(
            b"@prefix : <http://example.org/> . :a :knows :b . :b :name \"Bob\" .".as_slice(),
            RdfFormat::Turtle,
        )?;
        let QueryResults::Solutions(rows) = dataset.query("SELECT ?name WHERE { <http://example.org/a> <http://example.org/knows> ?b . ?b <http://example.org/name> ?name }")? else { panic!("expected SELECT rows") };
        assert_eq!(rows.collect::<Result<Vec<_>, _>>()?.len(), 1);
        dataset
            .update("INSERT DATA { <http://example.org/c> <http://example.org/name> \"Carol\" }")?;
    }
    let reopened = Dataset::open(root.path(), "../logical-name", false)?;
    assert_eq!(reopened.store().len()?, 3);
    let other = Dataset::open(root.path(), "other", true)?;
    assert!(other.store().is_empty()?);
    Ok(())
}

#[test]
fn malformed_ingest_rolls_back() -> anyhow::Result<()> {
    let dataset = Dataset::memory()?;
    assert!(
        dataset
            .ingest(
                b"<http://e/s> <http://e/p> <http://e/o> .\ninvalid".as_slice(),
                RdfFormat::NTriples
            )
            .is_err()
    );
    assert!(dataset.store().is_empty()?);
    Ok(())
}

#[test]
fn migration_preserves_source_and_empty_datasets() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let old = temp.path().join("old.redb");
    let new = temp.path().join("new");
    {
        let mut store = rdf_store::RedbQuadStorage::open(&old, "a")?;
        for quad in
            rdf_io::parse_nquads("_:a <urn:p> \"hello\"@en <urn:g> .\n_:a <urn:q> _:b <urn:g> .")?
        {
            store.insert(quad)?;
        }
    }
    drop(rdf_store::RedbQuadStorage::open(&old, "empty")?);
    let before = std::fs::read(&old)?;
    rdf_store::engine::migrate(&old, &new)?;
    assert_eq!(std::fs::read(&old)?, before);
    assert_eq!(Dataset::open(&new, "a", false)?.store().len()?, 2);
    assert!(Dataset::open(&new, "empty", false)?.store().is_empty()?);
    assert!(rdf_store::engine::migrate(&old, &new).is_err());
    assert_eq!(std::fs::read(&old)?, before);
    Ok(())
}

#[test]
fn semantic_comparison_handles_duplicates_blank_nodes_and_order() -> anyhow::Result<()> {
    use rdf_store::comparison::from_results;
    let data = Dataset::memory()?;
    let unordered = "SELECT ?x WHERE { VALUES ?x { 1 2 1 } }";
    let reordered = "SELECT ?x WHERE { VALUES ?x { 2 1 1 } }";
    assert_eq!(
        from_results(data.query(unordered)?, unordered)?,
        from_results(data.query(reordered)?, reordered)?
    );
    assert_ne!(
        from_results(data.query(unordered)?, unordered)?,
        from_results(data.query("SELECT ?x WHERE { VALUES ?x { 1 2 } }")?, unordered)?
    );
    let asc = "SELECT ?x WHERE { VALUES ?x { 1 2 } } ORDER BY ?x";
    let desc = "SELECT ?x WHERE { VALUES ?x { 1 2 } } ORDER BY DESC(?x)";
    assert_ne!(from_results(data.query(asc)?, asc)?, from_results(data.query(desc)?, desc)?);
    let bnodes = "SELECT (BNODE() AS ?b) WHERE { VALUES ?x { 1 2 } }";
    assert_eq!(
        from_results(data.query(bnodes)?, bnodes)?,
        from_results(data.query(bnodes)?, bnodes)?
    );
    Ok(())
}

#[test]
fn query_semantics_and_failed_update_are_stable() -> anyhow::Result<()> {
    let data = Dataset::memory()?;
    data.update("INSERT DATA { <urn:a> <urn:p> <urn:b> . <urn:b> <urn:p> <urn:c> . GRAPH <urn:g> { <urn:a> <urn:n> 1 } }; CREATE GRAPH <urn:empty>")?;
    for query in [
        "ASK { <urn:a> <urn:p>+ <urn:c> }",
        "ASK { GRAPH <urn:g> { <urn:a> <urn:n> 1 } }",
        "ASK { { SELECT (COUNT(*) AS ?n) WHERE { ?s <urn:p> ?o } } FILTER(?n = 2) }",
        "ASK { <urn:a> <urn:p> ?o OPTIONAL { ?o <urn:n> ?n FILTER(?n > 10) } FILTER(!BOUND(?n)) }",
    ] {
        assert!(matches!(data.query(query)?, QueryResults::Boolean(true)), "{query}");
    }
    assert!(data.update("INSERT DATA { <urn:z> <urn:p> 9 }; CREATE GRAPH <urn:g>").is_err());
    assert!(matches!(data.query("ASK { <urn:z> ?p ?o }")?, QueryResults::Boolean(false)));
    Ok(())
}

#[test]
fn network_operations_are_rejected_even_when_silent() -> anyhow::Result<()> {
    let data = Dataset::memory()?;
    for query in [
        "SELECT * WHERE { SERVICE SILENT <http://127.0.0.1:1/> { ?s ?p ?o } }",
        "ASK { FILTER EXISTS { SERVICE SILENT <http://127.0.0.1:1/> { ?s ?p ?o } } }",
        "SELECT * WHERE { VALUES ?x { 1 } LATERAL { SERVICE SILENT <http://127.0.0.1:1/> { ?s ?p ?o } } }",
    ] {
        assert!(data.query(query).is_err());
    }
    assert!(data.update("LOAD SILENT <http://127.0.0.1:1/>").is_err());
    Ok(())
}

#[test]
fn catalog_lists_and_deletes_datasets_with_isolation() -> anyhow::Result<()> {
    use rdf_store::engine::{delete_dataset, list_datasets, valid_dataset_name};
    let root = tempfile::tempdir()?;
    assert!(list_datasets(&root.path().join("missing"))?.is_empty());
    for name in ["alpha", "beta"] {
        let data = Dataset::open(root.path(), name, true)?;
        data.update(&format!("INSERT DATA {{ <urn:{name}> <urn:p> 1 }}"))?;
    }
    let listed = list_datasets(root.path())?;
    assert_eq!(listed.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);
    assert!(listed[0].identity.ends_with(&listed[0].id) && listed[0].id != listed[1].id);

    // A dataset that another handle has open cannot be deleted.
    let held = Dataset::open(root.path(), "alpha", false)?;
    assert!(delete_dataset(root.path(), "alpha").is_err());
    drop(held);

    let removed = delete_dataset(root.path(), "alpha")?;
    assert!(!root.path().join(&removed.id).exists());
    assert_eq!(list_datasets(root.path())?.len(), 1);
    assert!(Dataset::open(root.path(), "alpha", false).is_err());
    assert_eq!(Dataset::open(root.path(), "beta", false)?.store().len()?, 1);
    assert!(delete_dataset(root.path(), "alpha").is_err());

    assert!(valid_dataset_name("tenant-1.v2_x"));
    for bad in ["", ".hidden", "a/b", "../x", "sp ace", &"x".repeat(65)] {
        assert!(!valid_dataset_name(bad), "{bad}");
    }
    Ok(())
}
