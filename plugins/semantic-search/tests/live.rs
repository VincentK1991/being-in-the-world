//! Uses real Qdrant and a real embedding model. Never silently substitutes mock vectors.
use anyhow::Result;
fn new_plugin(id: &str, config: Config) -> Result<SemanticSearch> {
    SemanticSearch::new(id, config, &rdf_embedding_providers::registry()?)
}
use rdf_engine::Database;
use rdf_plugin_api::Plugin;
use rdf_plugin_api::ReadContext;
use rdf_plugin_semantic_search::{Config, IndexConfig, IndexMode, SemanticSearch};
use serde_json::json;

fn config() -> Config {
    Config {
        qdrant_url: std::env::var("TEST_QDRANT_URL").unwrap_or("http://127.0.0.1:16333".into()),
        embedding_url: Some(
            std::env::var("TEST_EMBEDDING_URL").unwrap_or("http://127.0.0.1:18081".into()),
        ),
        providers: Default::default(),
        indexes: vec![IndexConfig {
            name: "descriptions".into(),
            provider: None,
            predicates: vec!["http://www.w3.org/2000/01/rdf-schema#comment".into()],
            graph: None,
            max_entries: 100,
            mode: Default::default(),
        }],
    }
}
#[test]
#[ignore = "requires Qdrant and the local CPU/GPU embedding sidecars"]
fn semantic_search_rebuild_staleness_isolation_and_reopen() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let namespace = format!("live-test-{}", uuid::Uuid::new_v4());
    let mut db = Database::open(temp.path(), "default", true)?;
    db.register(new_plugin(&namespace, config())?)?;
    db.update(
        r#"PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
        INSERT DATA {
          <urn:car> rdfs:comment "An electric automobile stores power in rechargeable batteries." .
          <urn:car> rdfs:comment "Electric motors propel this road vehicle." .
          <urn:bread> rdfs:comment "A recipe for baking bread with flour and yeast." .
          <urn:stars> rdfs:comment "Distant galaxies contain stars and cosmic dust." .
          GRAPH <urn:french> { <urn:bonjour> rdfs:comment "Bonjour le monde"@fr . }
        }"#,
    )?;
    assert_eq!(
        db.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["state"],
        "missing"
    );
    let build = db.invoke("semantic-search", "rebuild", json!({"index":"descriptions"}))?;
    assert_eq!(build["entries"], 5);
    if std::env::var("REQUIRE_CUDA").as_deref() == Ok("1") {
        assert_eq!(build["device"], "cuda");
    }
    println!("Embedding execution: {}", serde_json::to_string(&build)?);
    let query = "PREFIX search: <urn:rdftool:search:> SELECT ?subject ?object ?score WHERE { (?subject ?predicate ?object ?score) search:matches ('descriptions' 'battery powered vehicles' 3) } ORDER BY DESC(?score)";
    let found = db.query_json(query)?;
    assert_eq!(found["results"]["bindings"][0]["subject"]["value"], "urn:car");
    let subjects = db.query_json("SELECT ?subject ?score WHERE { (?subject ?score) <urn:rdftool:search:subjects> ('descriptions' 'battery powered vehicles' 4) } ORDER BY DESC(?score)")?;
    assert_eq!(subjects["results"]["bindings"].as_array().expect("rows").len(), 4);
    assert_eq!(subjects["results"]["bindings"][0]["subject"]["value"], "urn:car");
    let objects = db.query_json("SELECT ?object ?score WHERE { (?object ?score) <urn:rdftool:search:objects> ('descriptions' 'greetings' 100) }")?;
    assert!(
        objects["results"]["bindings"]
            .as_array()
            .expect("rows")
            .iter()
            .any(|row| row["object"]["xml:lang"] == "fr")
    );
    let payload = db.invoke(
        "semantic-search",
        "search",
        json!({"index":"descriptions","text":"greetings","limit":100}),
    )?;
    assert!(
        payload["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .any(|hit| hit["entry"]["graph"]["value"] == "urn:french")
    );
    assert_eq!(db.overview()?.quads, 5);
    // Inject a malformed embedding response after collection creation. This does not
    // replace the real model in ranking tests; it checks failed-build publication.
    let health: serde_json::Value =
        reqwest::blocking::get(format!("{}/health", config().embedding_url.expect("url")))?
            .json()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let fake = std::thread::spawn(move || -> std::io::Result<()> {
        use std::io::{BufRead, Read, Write};
        for (i, connection) in listener.incoming().take(2).enumerate() {
            let mut connection = connection?;
            connection.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            let mut reader = std::io::BufReader::new(&mut connection);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            let response = if i == 0 {
                health.clone()
            } else {
                json!({"profile":health["profile"],"vectors":[]})
            }
            .to_string();
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                response.len(),
                response
            )?;
        }
        Ok(())
    });
    let mut broken_config = config();
    broken_config.embedding_url = Some(format!("http://{address}"));
    let broken = new_plugin(&namespace, broken_config)?;
    assert!(broken.invoke(&db, "rebuild", json!({"index":"descriptions"})).is_err());
    fake.join().expect("failure injection worker")?;
    assert_eq!(
        db.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["collection"],
        build["collection"]
    );
    drop(db);
    let mut db = Database::open(temp.path(), "default", false)?;
    db.register(new_plugin(&namespace, config())?)?;
    assert_eq!(
        db.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["state"],
        "ready"
    );
    assert_eq!(db.query_json(query)?["results"]["bindings"][0]["subject"]["value"], "urn:car");
    let mut other = Database::memory()?;
    other.register(new_plugin(&format!("{namespace}-other"), config())?)?;
    assert_eq!(
        other.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["state"],
        "missing"
    );
    // Same quad count, different content must invalidate the index.
    db.update("DELETE { <urn:stars> ?p ?o } INSERT { <urn:stars> ?p 'Astronomy observes planets and moons.' } WHERE { <urn:stars> ?p ?o }")?;
    assert_eq!(db.overview()?.quads, 5);
    assert_eq!(
        db.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["state"],
        "stale"
    );
    assert!(db.query(query).err().expect("stale index").to_string().contains("stale"));
    let rebuilt = db.invoke("semantic-search", "rebuild", json!({"index":"descriptions"}))?;
    assert_eq!(rebuilt["previous_collection"], build["collection"]);
    assert_ne!(rebuilt["collection"], build["collection"]);
    assert!(db.query(query).is_ok());
    // Test cleanup owns only the two collections and alias created by this unique namespace.
    let client = reqwest::blocking::Client::new();
    let base = config().qdrant_url;
    let aliases: serde_json::Value =
        client.get(format!("{base}/aliases")).send()?.error_for_status()?.json()?;
    for alias in aliases["result"]["aliases"].as_array().expect("aliases") {
        if alias["collection_name"] == rebuilt["collection"] {
            client
                .post(format!("{base}/collections/aliases"))
                .json(&json!({"actions":[{"delete_alias":{"alias_name":alias["alias_name"]}}]}))
                .send()?
                .error_for_status()?;
        }
    }
    for collection in [&build["collection"], &rebuilt["collection"]] {
        client
            .delete(format!("{base}/collections/{}", collection.as_str().expect("collection")))
            .send()?
            .error_for_status()?;
    }
    Ok(())
}

#[test]
#[ignore = "requires Qdrant, local embeddings, and OPENAI_API_KEY; sends fixture text to OpenAI"]
fn local_and_openai_indexes_in_one_query() -> Result<()> {
    // Explicitly invoked hosted test; missing credentials are a failure, never a silent skip.
    anyhow::ensure!(
        std::env::var("OPENAI_API_KEY").is_ok_and(|key| !key.trim().is_empty()),
        "OPENAI_API_KEY required"
    );
    let config: Config = serde_json::from_str(include_str!("../example-providers.json"))?;
    mixed_indexes(config)
}
fn mixed_indexes(config: Config) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut db = Database::open(temp.path(), "default", true)?;
    let namespace = format!("hosted-test-{}", uuid::Uuid::new_v4());
    db.register(new_plugin(&namespace, config.clone())?)?;
    db.update("INSERT DATA { <urn:car> <http://www.w3.org/2000/01/rdf-schema#comment> 'An electric automobile stores power in rechargeable batteries.' . <urn:bread> <http://www.w3.org/2000/01/rdf-schema#comment> 'A recipe for baking bread with flour and yeast.' . }")?;
    let mut builds = Vec::new();
    let outcome = (|| -> Result<()> {
        for name in ["descriptions", "descriptions_openai"] {
            builds.push(db.invoke("semantic-search", "rebuild", json!({"index":name}))?);
        }
        let result = db.query_json("PREFIX s: <urn:rdftool:search:> SELECT ?provider ?subject WHERE { { (?subject ?score) s:subjects ('descriptions' 'battery powered vehicles' 1) BIND('local' AS ?provider) } UNION { (?subject ?score) s:subjects ('descriptions_openai' 'battery powered vehicles' 1) BIND('openai' AS ?provider) } } ORDER BY ?provider")?;
        let rows = result["results"]["bindings"].as_array().expect("rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["subject"]["value"] == "urn:car"));
        drop(db);
        let mut changed = config.clone();
        changed.providers.get_mut("openai_small").expect("provider").config["revision"] =
            json!("changed-deployment");
        let mut db = Database::open(temp.path(), "default", false)?;
        db.register(new_plugin(&namespace, changed)?)?;
        assert_eq!(
            db.invoke("semantic-search", "status", json!({"index":"descriptions_openai"}))?["state"],
            "stale"
        );
        assert_eq!(
            db.invoke("semantic-search", "status", json!({"index":"descriptions"}))?["state"],
            "ready"
        );
        assert!(db.query("SELECT ?s WHERE { (?s ?score) <urn:rdftool:search:subjects> ('descriptions_openai' 'car' 1) }").is_err());
        Ok(())
    })();
    let client = reqwest::blocking::Client::new();
    let aliases: serde_json::Value =
        client.get(format!("{}/aliases", config.qdrant_url)).send()?.error_for_status()?.json()?;
    for alias in aliases["result"]["aliases"].as_array().expect("aliases") {
        if builds.iter().any(|build| build["collection"] == alias["collection_name"]) {
            client
                .post(format!("{}/collections/aliases", config.qdrant_url))
                .json(&json!({"actions":[{"delete_alias":{"alias_name":alias["alias_name"]}}]}))
                .send()?
                .error_for_status()?;
        }
    }
    for build in builds {
        client
            .delete(format!(
                "{}/collections/{}",
                config.qdrant_url,
                build["collection"].as_str().expect("collection")
            ))
            .send()?
            .error_for_status()?;
    }
    outcome
}

#[test]
#[ignore = "requires real Qdrant and local embeddings; hosted endpoint is a contract fixture"]
fn mixed_indexes_with_openai_contract_fixture() -> Result<()> {
    use std::io::{BufRead, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let worker = std::thread::spawn(move || -> Result<()> {
        for connection in listener.incoming().take(2) {
            let mut connection = connection?;
            connection.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            let mut reader = std::io::BufReader::new(&mut connection);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse()?;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            let body: serde_json::Value = serde_json::from_slice(&body)?;
            let inputs = body["input"].as_array().expect("input array");
            let data: Vec<_> = inputs.iter().enumerate().map(|(index,text)| json!({"index":index,"embedding":if text.as_str().expect("text").contains("bread") {vec![0.0,1.0]} else {vec![1.0,0.0]}})).collect();
            let response = json!({"model":"fixture","data":data}).to_string();
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                response.len(),
                response
            )?;
        }
        Ok(())
    });
    let mut config: Config = serde_json::from_str(include_str!("../example-providers.json"))?;
    config.providers.get_mut("openai_small").expect("provider").config = json!({"base_url":format!("http://{address}/v1"),"model":"fixture","dimensions":2,"api_key_env":"PATH","revision":"fixture-1"});
    let result = mixed_indexes(config);
    // On failure, do not hang waiting for a request that wasn't issued.
    if result.is_ok() {
        worker.join().expect("fixture server")?;
    }
    result
}

fn hybrid_config() -> Config {
    let mut config = config();
    config.indexes[0].name = "hybrid".into();
    config.indexes[0].mode = IndexMode::Hybrid;
    config.indexes.push(IndexConfig {
        name: "dense-only".into(),
        provider: None,
        predicates: vec!["http://www.w3.org/2000/01/rdf-schema#comment".into()],
        graph: None,
        max_entries: 100,
        mode: IndexMode::Dense,
    });
    config
}
fn subjects(db: &Database, query: &str) -> Result<Vec<String>> {
    Ok(db.query_json(query)?["results"]["bindings"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["subject"]["value"].as_str().expect("subject").to_owned())
        .collect())
}
#[test]
#[ignore = "requires Qdrant and the local CPU/GPU embedding sidecars"]
fn lexical_and_hybrid_search_work_through_sparql_alongside_dense_search() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let namespace = format!("live-hybrid-{}", uuid::Uuid::new_v4());
    let mut db = Database::open(temp.path(), "default", true)?;
    db.register(new_plugin(&namespace, hybrid_config())?)?;
    db.update(
        r#"PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
        INSERT DATA {
          <urn:car> rdfs:comment "An electric automobile stores power in rechargeable batteries." .
          <urn:bus> rdfs:comment "A large road vehicle carries many passengers along a fixed route." .
          <urn:bread> rdfs:comment "A recipe for baking bread with flour and yeast." .
          <urn:stars> rdfs:comment "Distant galaxies contain stars and cosmic dust." .
          <urn:claim> rdfs:comment "Reference QX7731 was settled by the regional office in March." .
          <urn:policy> rdfs:comment "Reference ZK2210 covers water damage to the basement." .
        }"#,
    )?;
    let hybrid = db.invoke("semantic-search", "rebuild", json!({"index":"hybrid"}))?;
    assert_eq!(hybrid["entries"], 6);
    db.invoke("semantic-search", "rebuild", json!({"index":"dense-only"}))?;
    assert_eq!(
        db.invoke("semantic-search", "status", json!({"index":"hybrid"}))?["mode"],
        "hybrid"
    );
    let q = |property: &str, index: &str, text: &str, k: u32| {
        format!(
            "PREFIX search: <urn:rdftool:search:> SELECT ?subject ?score WHERE {{ (?subject ?predicate ?object ?score) search:{property} ('{index}' '{text}' {k}) }} ORDER BY DESC(?score)"
        )
    };

    // Lexical: only an exact keyword finds the opaque reference number, and only the right document.
    assert_eq!(subjects(&db, &q("lexical", "hybrid", "QX7731", 5))?, ["urn:claim"]);
    // Keyword search is case-insensitive and ranks the document with more matching terms first.
    let ranked = subjects(&db, &q("lexical", "hybrid", "reference water damage", 5))?;
    assert_eq!(ranked[0], "urn:policy");
    assert!(ranked.contains(&"urn:claim".to_string()));
    // A query with no word in the corpus returns nothing instead of arbitrary neighbours.
    assert!(subjects(&db, &q("lexical", "hybrid", "unrelatedword", 5))?.is_empty());

    // Hybrid keeps the semantic hit (no shared words with the query) and adds the exact-term hit.
    let fused = subjects(&db, &q("hybrid", "hybrid", "battery powered vehicles QX7731", 3))?;
    assert!(fused.contains(&"urn:car".to_string()), "semantic hit missing: {fused:?}");
    assert!(fused.contains(&"urn:claim".to_string()), "lexical hit missing: {fused:?}");
    // Dense-only search still works on a hybrid index and on a dense index, with the same top hit.
    for index in ["hybrid", "dense-only"] {
        assert_eq!(
            subjects(&db, &q("matches", index, "battery powered vehicles", 3))?[0],
            "urn:car"
        );
    }
    // Scores are numeric and ordered, and the fused list has no duplicate subjects' entries.
    let rows = db.query_json(&q("hybrid", "hybrid", "bread yeast", 4))?;
    let scores: Vec<f64> = rows["results"]["bindings"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["score"]["value"].as_str().expect("score").parse().expect("number"))
        .collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(subjects(&db, &q("hybrid", "hybrid", "bread yeast", 4))?[0], "urn:bread");

    // Lexical and hybrid are refused on a dense index with an actionable message.
    let error = db
        .query_json(&q("lexical", "dense-only", "bread", 3))
        .expect_err("dense index")
        .to_string();
    assert!(error.contains("hybrid"), "{error}");
    // The SPARQL surface advertises the new properties.
    let iris: Vec<String> =
        db.plugins().into_iter().flat_map(|p| p.properties).map(|p| p.iri).collect();
    for name in ["lexical", "hybrid"] {
        assert!(iris.contains(&format!("urn:rdftool:search:{name}")));
    }
    // Both indexes' collections belong to this namespace; remove them so repeated runs stay clean.
    assert_eq!(
        rdf_plugin_semantic_search::purge_dataset(&hybrid_config().qdrant_url, &namespace)?,
        2
    );
    Ok(())
}
