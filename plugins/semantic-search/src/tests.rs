fn new_plugin(id: &str, config: Config) -> Result<SemanticSearch> {
    SemanticSearch::new(id, config, &rdf_embedding_providers::registry()?)
}
use super::*;
fn config() -> Config {
    Config {
        qdrant_url: "http://127.0.0.1:16333".into(),
        embedding_url: Some("http://127.0.0.1:18081".into()),
        providers: Default::default(),
        indexes: vec![IndexConfig {
            name: "test".into(),
            provider: None,
            predicates: vec![],
            graph: None,
            max_entries: 100,
            mode: Default::default(),
        }],
    }
}
struct Corpus(Value);
impl ReadContext for Corpus {
    fn query_json(&self, query: &str) -> Result<Value> {
        assert!(query.contains("isIRI(?subject)"));
        assert!(query.contains("LIMIT 101"));
        Ok(self.0.clone())
    }
}
fn entry(text: &str) -> Value {
    json!({"subject":{"type":"uri","value":"urn:s"}, "predicate":{"type":"uri","value":"urn:p"}, "object":{"type":"literal","value":text,"xml:lang":"fr"}, "graph":{"type":"uri","value":"urn:g"}})
}
#[test]
fn config_rejects_remote_urls_and_bad_rdf_iris() -> Result<()> {
    for url in [
        "https://127.0.0.1:6333",
        "http://example.org",
        "http://localhost:6333",
        "http://127.0.0.1/path",
        "http://user@127.0.0.1",
        "http://127.0.0.1?token=x",
    ] {
        let mut c = config();
        c.qdrant_url = url.into();
        assert!(new_plugin("id", c).is_err(), "{url}");
    }
    let mut c = config();
    c.indexes[0].predicates.push("urn:bad> } UNION {".into());
    assert!(new_plugin("id", c).is_err());
    let mut c = config();
    c.indexes.push(c.indexes[0].clone());
    assert!(new_plugin("id", c).is_err());
    assert!(new_plugin("id", config()).is_ok());
    Ok(())
}
#[test]
fn fingerprint_is_order_independent_and_detects_same_count_changes() -> Result<()> {
    let plugin = new_plugin("dataset", config())?;
    let index = &plugin.state.config.indexes[0];
    let first = Corpus(json!({"results":{"bindings":[entry("bonjour"),entry("salut")]}}));
    let reversed = Corpus(json!({"results":{"bindings":[entry("salut"),entry("bonjour")]}}));
    let changed = Corpus(json!({"results":{"bindings":[entry("bonsoir"),entry("salut")]}}));
    let (rows, digest) = plugin.state.corpus(&first, index)?;
    assert_eq!(digest, plugin.state.corpus(&reversed, index)?.1);
    assert_ne!(digest, plugin.state.corpus(&changed, index)?.1);
    assert_eq!(
        rows[0].object.rdf(),
        RdfValue::Literal {
            value: "bonjour".into(),
            datatype: LANG.into(),
            language: Some("fr".into())
        }
    );
    assert_eq!(rows[0].graph.as_ref().expect("graph").value, "urn:g");
    Ok(())
}
#[test]
fn dataset_namespaces_and_configuration_fingerprints_are_separate() -> Result<()> {
    let a = new_plugin("a", config())?;
    let b = new_plugin("b", config())?;
    assert_ne!(a.state.alias(&config().indexes[0]), b.state.alias(&config().indexes[0]));
    assert_eq!(
        a.state.alias(&config().indexes[0]),
        new_plugin("a", config())?.state.alias(&config().indexes[0])
    );
    let mut changed = config().indexes[0].clone();
    changed.graph = Some("default".into());
    assert_ne!(State::config_digest(&changed)?, State::config_digest(&config().indexes[0])?);
    Ok(())
}
#[test]
fn invalid_search_arguments_fail_before_contacting_sidecars() -> Result<()> {
    let plugin = new_plugin("id", config())?;
    let context = Corpus(Value::Null);
    for input in [
        json!({"index":"test","text":"hello","limit":"ten"}),
        json!({"index":"test","text":"hello","limit":0}),
        json!({"index":"test","text":"hello","limit":101}),
        json!({"index":"test","text":""}),
        json!({"index":"test","text":"hello","mode":false}),
        json!({"index":"test","text":"hello","unexpected":true}),
    ] {
        assert!(plugin.invoke(&context, "search", input).is_err());
    }
    Ok(())
}

#[test]
fn externally_registered_provider_routes_per_index_without_core_changes() -> Result<()> {
    use rdf_embedding_api::{EmbeddingProvider, Profile, ProviderFactory, ProviderRegistry};
    struct Factory;
    struct Provider(String);
    impl ProviderFactory for Factory {
        fn create(&self, value: &Value) -> Result<Arc<dyn EmbeddingProvider>> {
            Ok(Arc::new(Provider(value["model"].as_str().context("model")?.into())))
        }
    }
    impl EmbeddingProvider for Provider {
        fn health(&self) -> Result<Health> {
            Ok(Health {
                device: "test".into(),
                profile: Profile {
                    provider: "third-party".into(),
                    model: self.0.clone(),
                    dimension: 2,
                    identity: json!({"revision":1}),
                },
            })
        }
        fn embed(&self, texts: &[String], _: &Profile) -> Result<Vec<Vec<f32>>> {
            Ok(vec![vec![1.0, 0.0]; texts.len()])
        }
    }
    let mut registry = ProviderRegistry::default();
    registry.register("third-party", Factory)?;
    assert!(registry.register("third-party", Factory).is_err());
    let mut c = config();
    c.embedding_url = None;
    c.providers.insert(
        "a".into(),
        ProviderConfig { kind: "third-party".into(), config: json!({"model":"model-a"}) },
    );
    c.providers.insert(
        "b".into(),
        ProviderConfig { kind: "third-party".into(), config: json!({"model":"model-b"}) },
    );
    c.indexes[0].provider = Some("a".into());
    let mut second = c.indexes[0].clone();
    second.name = "second".into();
    second.provider = Some("b".into());
    c.indexes.push(second);
    let plugin = SemanticSearch::new("dataset", c.clone(), &registry)?;
    let a = plugin.state.health(&c.indexes[0])?;
    let b = plugin.state.health(&c.indexes[1])?;
    assert_eq!(a.profile.model, "model-a");
    assert_eq!(b.profile.model, "model-b");
    assert_ne!(a.profile, b.profile);
    c.indexes[0].provider = Some("missing".into());
    assert!(SemanticSearch::new("dataset", c, &registry).is_err());
    Ok(())
}

#[test]
fn purge_prefix_matches_the_aliases_a_dataset_owns() -> Result<()> {
    let plugin = new_plugin("tenant-a", config())?;
    let alias = plugin.state.alias(&config().indexes[0]);
    assert!(alias.starts_with(&crate::dataset_prefix("tenant-a")));
    assert!(!alias.starts_with(&crate::dataset_prefix("tenant-b")));
    assert!(purge_dataset("http://example.com:6333", "tenant-a").is_err(), "non-loopback refused");
    assert!(purge_dataset("http://127.0.0.1:6333", "").is_err());
    Ok(())
}
#[test]
fn tokenizer_lowercases_and_splits_on_non_alphanumerics() {
    assert_eq!(
        tokenize("Claim QX-7731, settled; İstanbul!"),
        ["claim", "qx", "7731", "settled", "i̇stanbul"]
    );
    assert!(tokenize("  ... --- ").is_empty());
}
#[test]
fn bm25_weights_saturate_with_frequency_and_penalise_long_text() {
    let weight = |text: &str, term: &str| {
        let sparse = sparse_vector(text, Some(4.0));
        let at = sparse["indices"]
            .as_array()
            .expect("indices")
            .iter()
            .position(|i| *i == json!(term_id(term)))
            .expect("term present");
        sparse["values"][at].as_f64().expect("weight")
    };
    assert!(weight("fox fox fox fox", "fox") > weight("fox", "fox"));
    assert!(
        weight("fox fox fox fox", "fox") < 4.0 * weight("fox", "fox"),
        "term frequency saturates"
    );
    assert!(
        weight("fox a b c d e f g", "fox") < weight("fox a b c", "fox"),
        "longer text scores lower"
    );
    let query = sparse_vector("fox fox den", None);
    assert_eq!(query["values"], json!([1.0, 1.0]), "queries weight each distinct term once");
    assert!(!has_terms(&sparse_vector("!!!", None)));
}
#[test]
fn dense_configuration_digest_is_unchanged_by_the_mode_field() -> Result<()> {
    let mut index = config().indexes.remove(0);
    let dense = State::config_digest(&index);
    assert!(!serde_json::to_string(&index)?.contains("mode"), "dense indexes serialize as before");
    index.mode = IndexMode::Hybrid;
    assert_ne!(dense?, State::config_digest(&index)?, "switching mode forces a rebuild");
    Ok(())
}
#[test]
fn lexical_and_hybrid_search_require_a_hybrid_index() -> Result<()> {
    let plugin = new_plugin("dataset", config())?;
    let error = plugin
        .state
        .search(&Corpus(json!({})), "test", "anything", 3, "lexical")
        .expect_err("dense index");
    assert!(error.to_string().contains("\"mode\": \"hybrid\""), "{error}");
    let mut c = config();
    c.indexes[0].mode = IndexMode::Hybrid;
    let parsed: Config = serde_json::from_str(&serde_json::to_string(&c)?)?;
    assert_eq!(parsed.indexes[0].mode, IndexMode::Hybrid);
    assert!(
        serde_json::from_str::<Config>(
            r#"{"qdrant_url":"http://127.0.0.1:1","indexes":[{"name":"a","mode":"sparse"}]}"#
        )
        .is_err()
    );
    Ok(())
}
