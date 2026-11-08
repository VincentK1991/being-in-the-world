//! Semantic property plugin: Qdrant owns vector storage/search; registered providers embed text.
use anyhow::{Context, Result, bail, ensure};
use rdf_embedding_api::{EmbeddingProvider, Health, Profile as ModelProfile, ProviderRegistry};
use rdf_plugin_api::{
    Manifest, Plugin, PropertyDescriptor, PropertyFunction, RdfValue, ReadContext, RowSink,
    SDK_VERSION,
};
use reqwest::{Method, blocking::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

const PREFIX: &str = "urn:rdftool:search:";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const LANG: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const SCHEMA: u32 = 2;
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub qdrant_url: String,
    #[serde(default)]
    pub embedding_url: Option<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    pub indexes: Vec<IndexConfig>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub kind: String,
    pub config: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    pub name: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub predicates: Vec<String>,
    /// None means all graphs, "default" selects the default graph, otherwise an IRI.
    #[serde(default)]
    pub graph: Option<String>,
    #[serde(default = "max_entries")]
    pub max_entries: usize,
    /// `dense` stores embeddings only. `hybrid` also stores BM25 term vectors so lexical and
    /// fused (dense + lexical) search work on the same index.
    #[serde(default, skip_serializing_if = "IndexMode::is_dense")]
    pub mode: IndexMode,
}
fn max_entries() -> usize {
    100_000
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    #[default]
    Dense,
    Hybrid,
}
impl IndexMode {
    // Skipped when serializing so existing dense indexes keep their configuration digest.
    fn is_dense(&self) -> bool {
        *self == Self::Dense
    }
}

// ---- Lexical (BM25) side of hybrid search ----
// Qdrant stores sparse vectors and applies the inverse document frequency itself when the sparse
// vector is declared with `modifier: idf`. The plugin supplies the term-frequency half: tokens are
// hashed to stable u32 ids and documents get saturated, length-normalised term weights.
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const DENSE: &str = "dense";
const LEXICAL: &str = "lexical";
/// Lowercased alphanumeric words. Deliberately simple and language-neutral: no stemming or stop words.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && word.len() <= 64)
        .map(str::to_lowercase)
        .collect()
}
fn term_id(token: &str) -> u32 {
    let hash = Sha256::digest(token.as_bytes());
    u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]])
}
/// Sparse vector as parallel `indices`/`values`, with one entry per distinct term.
/// Documents use BM25 term weights (`avg_len` is the corpus average length); queries weight every
/// distinct term 1 so repeated words do not dominate.
fn sparse_vector(text: &str, avg_len: Option<f64>) -> Value {
    let tokens = tokenize(text);
    let mut counts: BTreeMap<u32, f64> = BTreeMap::new();
    for token in &tokens {
        *counts.entry(term_id(token)).or_default() += 1.0;
    }
    let length = tokens.len() as f64;
    let (indices, values): (Vec<u32>, Vec<f64>) = counts
        .into_iter()
        .map(|(id, tf)| match avg_len {
            Some(avg) => (
                id,
                tf * (BM25_K1 + 1.0)
                    / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * length / avg.max(1.0))),
            ),
            None => (id, 1.0),
        })
        .unzip();
    json!({"indices": indices, "values": values})
}
fn has_terms(sparse: &Value) -> bool {
    sparse["indices"].as_array().is_some_and(|indices| !indices.is_empty())
}
fn local_url(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value)?;
    ensure!(
        url.scheme() == "http"
            && url.host_str().is_some_and(|host| host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "sidecar URL must be a loopback HTTP origin, e.g. http://127.0.0.1:16333"
    );
    Ok(value.trim_end_matches('/').into())
}
fn iri(value: &str) -> Result<()> {
    oxiri::Iri::parse(value).context("invalid RDF IRI")?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Term {
    kind: String,
    value: String,
    datatype: Option<String>,
    language: Option<String>,
}
impl Term {
    fn binding(value: &Value) -> Result<Self> {
        let kind = value["type"].as_str().context("missing RDF term type")?;
        ensure!(
            kind == "uri" || kind == "literal" || kind == "bnode",
            "invalid RDF term in corpus"
        );
        Ok(Self {
            kind: kind.into(),
            value: value["value"].as_str().context("missing RDF value")?.into(),
            datatype: value["datatype"].as_str().map(str::to_owned),
            language: value["xml:lang"].as_str().map(str::to_owned),
        })
    }
    fn rdf(&self) -> RdfValue {
        if self.kind == "uri" {
            RdfValue::Iri(self.value.clone())
        } else {
            RdfValue::Literal {
                value: self.value.clone(),
                datatype: self.datatype.clone().unwrap_or_else(|| {
                    if self.language.is_some() { LANG.into() } else { format!("{XSD}string") }
                }),
                language: self.language.clone(),
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    subject: Term,
    predicate: Term,
    object: Term,
    graph: Option<Term>,
}
#[derive(Serialize, Deserialize)]
struct IndexManifest {
    schema: u32,
    dataset: String,
    config_digest: String,
    source_digest: String,
    entries: usize,
    profile: ModelProfile,
}

struct State {
    config: Config,
    dataset: String,
    client: Client,
    rebuild: Mutex<()>,
    providers: BTreeMap<String, Arc<dyn EmbeddingProvider>>,
}
#[derive(Clone)]
pub struct SemanticSearch {
    state: Arc<State>,
}
impl SemanticSearch {
    pub fn new(
        dataset_identity: &str,
        mut config: Config,
        registry: &ProviderRegistry,
    ) -> Result<Self> {
        ensure!(!dataset_identity.is_empty(), "dataset identity must not be empty");
        config.qdrant_url = local_url(&config.qdrant_url)?;
        ensure!(
            config.embedding_url.is_none() || config.providers.is_empty(),
            "legacy embedding_url cannot be combined with providers"
        );
        if let Some(url) = &config.embedding_url {
            config.providers.insert(
                "legacy-local".into(),
                ProviderConfig { kind: "local".into(), config: json!({"url":url}) },
            );
            for index in &mut config.indexes {
                ensure!(
                    index.provider.is_none(),
                    "legacy embedding_url cannot be combined with index provider"
                );
                index.provider = Some("legacy-local".into());
            }
        }
        let mut providers = BTreeMap::new();
        for (name, provider) in &config.providers {
            ensure!(!name.is_empty(), "empty provider name");
            providers.insert(name.clone(), registry.create(&provider.kind, &provider.config)?);
        }
        ensure!(!config.indexes.is_empty(), "configure at least one semantic index");
        let mut names = HashSet::new();
        for index in &config.indexes {
            ensure!(
                index.provider.as_ref().is_some_and(|name| providers.contains_key(name)),
                "index must reference a configured provider"
            );
            ensure!(
                !index.name.is_empty()
                    && index.name.len() <= 64
                    && index
                        .name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                    && names.insert(index.name.clone()),
                "invalid or duplicate semantic index name"
            );
            ensure!(
                (1..=1_000_000).contains(&index.max_entries),
                "max_entries must be 1 to 1000000"
            );
            for predicate in &index.predicates {
                iri(predicate)?;
            }
            ensure!(
                index.predicates.iter().collect::<HashSet<_>>().len() == index.predicates.len(),
                "duplicate predicate in semantic configuration"
            );
            if let Some(graph) = &index.graph
                && graph != "default"
            {
                iri(graph)?;
            }
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build()?;
        Ok(Self {
            state: Arc::new(State {
                config,
                dataset: digest(dataset_identity.as_bytes()),
                client,
                rebuild: Mutex::new(()),
                providers,
            }),
        })
    }
}

/// Prefix shared by every Qdrant alias and collection owned by one dataset.
fn dataset_prefix(dataset_identity: &str) -> String {
    format!("rdfvs_{}_", &digest(dataset_identity.as_bytes())[..32])
}
/// Deletes every Qdrant collection (and with it, its aliases) owned by a dataset. Call after the
/// dataset has been removed; the plugin keeps no other state outside the RDF store.
/// Returns the number of collections deleted.
pub fn purge_dataset(qdrant_url: &str, dataset_identity: &str) -> Result<usize> {
    ensure!(!dataset_identity.is_empty(), "dataset identity must not be empty");
    let base = local_url(qdrant_url)?;
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()?;
    let listing: Value = client
        .get(format!("{base}/collections"))
        .send()
        .context("local semantic sidecar unavailable")?
        .error_for_status()?
        .json()?;
    let prefix = dataset_prefix(dataset_identity);
    let owned: Vec<&str> = listing["result"]["collections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str())
        .filter(|name| name.starts_with(&prefix))
        .collect();
    for name in &owned {
        client.delete(format!("{base}/collections/{name}")).send()?.error_for_status()?;
    }
    Ok(owned.len())
}

#[cfg(test)]
mod tests;
impl State {
    fn index(&self, name: &str) -> Result<&IndexConfig> {
        self.config
            .indexes
            .iter()
            .find(|index| index.name == name)
            .context("unknown semantic index")
    }
    fn alias(&self, index: &IndexConfig) -> String {
        format!("rdfvs_{}_{}", &self.dataset[..32], &digest(index.name.as_bytes())[..16])
    }
    fn request(
        &self,
        method: Method,
        base: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self.client.request(method, format!("{base}{path}"));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().context("local semantic sidecar unavailable")?;
        let status = response.status();
        let text = response.text()?;
        ensure!(
            status.is_success(),
            "semantic sidecar {path} returned {status}: {}",
            &text.chars().take(1000).collect::<String>()
        );
        serde_json::from_str(&text).context("invalid sidecar JSON")
    }
    fn qdrant(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.request(method, &self.config.qdrant_url, path, body)
    }
    fn provider(&self, index: &IndexConfig) -> &dyn EmbeddingProvider {
        self.providers[index.provider.as_ref().expect("validated provider")].as_ref()
    }
    fn health(&self, index: &IndexConfig) -> Result<Health> {
        let health = self.provider(index).health()?;
        ensure!(
            (1..=65536).contains(&health.profile.dimension) && !health.profile.model.is_empty(),
            "invalid embedding profile"
        );
        Ok(health)
    }
    fn embed(
        &self,
        index: &IndexConfig,
        texts: &[String],
        expected: &ModelProfile,
    ) -> Result<Vec<Vec<f32>>> {
        let vectors = self.provider(index).embed(texts, expected)?;
        rdf_embedding_api::validate(&vectors, texts.len(), expected)?;
        Ok(vectors)
    }
    fn corpus(
        &self,
        context: &dyn ReadContext,
        index: &IndexConfig,
    ) -> Result<(Vec<Entry>, String)> {
        let graph_pattern = match index.graph.as_deref() {
            Some("default") => "?subject ?predicate ?object .".into(),
            Some(graph) => format!("GRAPH <{graph}> {{ ?subject ?predicate ?object }} BIND(<{graph}> AS ?graph)"),
            None => "{ ?subject ?predicate ?object } UNION { GRAPH ?graph { ?subject ?predicate ?object } }".into(),
        };
        let predicates = if index.predicates.is_empty() {
            String::new()
        } else {
            format!(
                "VALUES ?predicate {{ {} }}",
                index.predicates.iter().map(|p| format!("<{p}>")).collect::<Vec<_>>().join(" ")
            )
        };
        let query = format!(
            "SELECT ?subject ?predicate ?object ?graph WHERE {{ {{ {graph_pattern} }} {predicates} FILTER(isIRI(?subject) && isLiteral(?object) && (datatype(?object) = <{XSD}string> || datatype(?object) = <{LANG}>)) }} LIMIT {}",
            index.max_entries + 1
        );
        let result = context.query_json(&query)?;
        let rows =
            result["results"]["bindings"].as_array().context("invalid corpus query results")?;
        ensure!(rows.len() <= index.max_entries, "semantic corpus exceeds configured max_entries");
        let mut entries = Vec::new();
        for row in rows {
            let entry = Entry {
                subject: Term::binding(&row["subject"])?,
                predicate: Term::binding(&row["predicate"])?,
                object: Term::binding(&row["object"])?,
                graph: row.get("graph").map(Term::binding).transpose()?,
            };
            if entry.object.value.trim().is_empty() {
                continue;
            }
            ensure!(entry.object.value.len() <= 65536, "literal exceeds 64 KiB embedding limit");
            entries.push((serde_json::to_string(&entry)?, entry));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let digest =
            digest(&serde_json::to_vec(&entries.iter().map(|(text, _)| text).collect::<Vec<_>>())?);
        Ok((entries.into_iter().map(|(_, entry)| entry).collect(), digest))
    }
    fn collection(&self, alias: &str) -> Result<Option<String>> {
        let result = self.qdrant(Method::GET, "/aliases", None)?;
        let aliases = result["result"]["aliases"].as_array().context("invalid Qdrant aliases")?;
        Ok(aliases
            .iter()
            .find(|a| a["alias_name"] == alias)
            .and_then(|a| a["collection_name"].as_str())
            .map(str::to_owned))
    }
    fn manifest(&self, collection: &str) -> Result<IndexManifest> {
        let result =
            self.qdrant(Method::GET, &format!("/collections/{collection}/points/0"), None)?;
        serde_json::from_value(result["result"]["payload"]["manifest"].clone())
            .context("index metadata is missing or invalid; rebuild")
    }
    fn config_digest(index: &IndexConfig) -> Result<String> {
        Ok(digest(&serde_json::to_vec(index)?))
    }
    fn rebuild(&self, context: &dyn ReadContext, name: &str) -> Result<Value> {
        let _guard = self
            .rebuild
            .try_lock()
            .map_err(|_| anyhow::anyhow!("semantic rebuild already running"))?;
        let index = self.index(name)?;
        let health = self.health(index)?;
        let (entries, source_digest) = self.corpus(context, index)?;
        let alias = self.alias(index);
        let collection = format!("{alias}_{}", uuid::Uuid::new_v4().simple());
        let dense =
            json!({"size": health.profile.dimension, "distance": "Cosine", "on_disk": true});
        let layout = match index.mode {
            IndexMode::Dense => json!({"vectors": dense, "on_disk_payload": true}),
            IndexMode::Hybrid => {
                json!({"vectors": {DENSE: dense}, "sparse_vectors": {LEXICAL: {"modifier": "idf"}}, "on_disk_payload": true})
            }
        };
        self.qdrant(Method::PUT, &format!("/collections/{collection}"), Some(layout))?;
        let average_length = if entries.is_empty() {
            1.0
        } else {
            entries.iter().map(|entry| tokenize(&entry.object.value).len()).sum::<usize>() as f64
                / entries.len() as f64
        };
        let stage = (|| -> Result<()> {
            for field in ["kind", "subject_key", "object_key"] {
                self.qdrant(
                    Method::PUT,
                    &format!("/collections/{collection}/index?wait=true"),
                    Some(json!({"field_name": field, "field_schema": "keyword"})),
                )?;
            }
            for batch in entries.chunks(32) {
                let vectors = self.embed(
                    index,
                    &batch.iter().map(|entry| entry.object.value.clone()).collect::<Vec<_>>(),
                    &health.profile,
                )?;
                let points: Vec<_> = batch.iter().zip(vectors).map(|(entry, vector)| {
                    let hash = Sha256::digest(serde_json::to_vec(entry).expect("serializable entry"));
                    let mut id = [0;16]; id.copy_from_slice(&hash[..16]);
                    let vector = match index.mode {
                        IndexMode::Dense => json!(vector),
                        IndexMode::Hybrid => json!({DENSE: vector, LEXICAL: sparse_vector(&entry.object.value, Some(average_length))}),
                    };
                    json!({"id": uuid::Uuid::from_bytes(id).to_string(), "vector": vector, "payload": {"kind": "literal", "subject_key": entry.subject.value, "object_key": digest(&serde_json::to_vec(&entry.object).expect("serializable term")), "entry": entry}})
                }).collect();
                self.qdrant(
                    Method::PUT,
                    &format!("/collections/{collection}/points?wait=true"),
                    Some(json!({"points": points})),
                )?;
            }
            let manifest = IndexManifest {
                schema: SCHEMA,
                dataset: self.dataset.clone(),
                config_digest: Self::config_digest(index)?,
                source_digest: source_digest.clone(),
                entries: entries.len(),
                profile: health.profile.clone(),
            };
            // Metadata point is excluded from every search by a mandatory kind filter.
            self.qdrant(Method::PUT, &format!("/collections/{collection}/points?wait=true"), Some(json!({"points": [{"id": 0, "vector": match index.mode { IndexMode::Dense => json!(vec![0.0; health.profile.dimension]), IndexMode::Hybrid => json!({DENSE: vec![0.0; health.profile.dimension]}) }, "payload": {"kind": "manifest", "manifest": manifest}}]})))?;
            let count = self.qdrant(
                Method::POST,
                &format!("/collections/{collection}/points/count"),
                Some(json!({"exact": true})),
            )?;
            ensure!(
                count["result"]["count"].as_u64() == Some(entries.len() as u64 + 1),
                "Qdrant index count verification failed"
            );
            ensure!(
                self.corpus(context, index)?.1 == source_digest,
                "RDF changed during rebuild; retry"
            );
            Ok(())
        })();
        if let Err(error) = stage {
            let _ = self.qdrant(Method::DELETE, &format!("/collections/{collection}"), None);
            return Err(error);
        }
        let previous = self.collection(&alias)?;
        let mut actions = Vec::new();
        if previous.is_some() {
            actions.push(json!({"delete_alias": {"alias_name": alias}}));
        }
        actions.push(json!({"create_alias": {"alias_name": alias, "collection_name": collection}}));
        // Never delete the staged collection on an ambiguous publish timeout: it may be active.
        self.qdrant(Method::POST, "/collections/aliases", Some(json!({"actions": actions})))?;
        Ok(
            json!({"index": name, "collection": collection, "previous_collection": previous, "entries": entries.len(), "source_digest": source_digest, "device": health.device, "profile": health.profile}),
        )
    }
    fn status(&self, context: &dyn ReadContext, name: &str) -> Result<Value> {
        let index = self.index(name)?;
        let alias = self.alias(index);
        let Some(collection) = self.collection(&alias)? else {
            return Ok(json!({"index": name, "state": "missing", "alias": alias}));
        };
        let manifest = self.manifest(&collection)?;
        let source_digest = self.corpus(context, index)?.1;
        let health = self.health(index)?;
        let current = manifest.schema == SCHEMA
            && manifest.dataset == self.dataset
            && manifest.config_digest == Self::config_digest(index)?
            && manifest.source_digest == source_digest
            && manifest.profile == health.profile;
        Ok(
            json!({"index": name, "state": if current { "ready" } else { "stale" }, "mode": index.mode, "collection": collection, "entries": manifest.entries, "device": health.device, "profile": health.profile}),
        )
    }
    fn search(
        &self,
        context: &dyn ReadContext,
        name: &str,
        text: &str,
        limit: usize,
        mode: &str,
    ) -> Result<Value> {
        ensure!(
            !text.trim().is_empty() && text.len() <= 65536,
            "search text must be nonempty and at most 64 KiB"
        );
        ensure!((1..=100).contains(&limit), "search limit must be 1 to 100");
        ensure!(
            ["matches", "subjects", "objects", "lexical", "hybrid"].contains(&mode),
            "unknown search mode"
        );
        let index = self.index(name)?;
        ensure!(
            index.mode == IndexMode::Hybrid || !["lexical", "hybrid"].contains(&mode),
            "lexical and hybrid search need an index with \"mode\": \"hybrid\"; set it and rebuild"
        );
        let collection =
            self.collection(&self.alias(index))?.context("semantic index missing; run rebuild")?;
        let manifest = self.manifest(&collection)?;
        let profile = self.health(index)?.profile;
        ensure!(
            manifest.schema == SCHEMA
                && manifest.dataset == self.dataset
                && manifest.config_digest == Self::config_digest(index)?
                && manifest.profile == profile,
            "semantic index configuration/model mismatch; rebuild required"
        );
        ensure!(
            self.corpus(context, index)?.1 == manifest.source_digest,
            "semantic index is stale after RDF changes; rebuild required"
        );
        let must = json!({"must": [{"key": "kind", "match": {"value": "literal"}}]});
        let sparse = sparse_vector(text, None);
        let dense = |this: &Self| -> Result<Value> {
            Ok(json!(this.embed(index, &[text.into()], &profile)?.remove(0)))
        };
        let hybrid = index.mode == IndexMode::Hybrid;
        let mut body = match mode {
            "lexical" => {
                ensure!(has_terms(&sparse), "search text has no searchable words");
                json!({"query": sparse, "using": LEXICAL, "limit": limit, "with_payload": true, "filter": must})
            }
            "hybrid" => {
                // Each branch over-fetches so fusion can promote hits that rank well in only one list.
                let depth = (limit * 4).clamp(20, 200);
                let mut branches = vec![
                    json!({"query": dense(self)?, "using": DENSE, "limit": depth, "filter": must}),
                ];
                if has_terms(&sparse) {
                    branches.push(
                        json!({"query": sparse, "using": LEXICAL, "limit": depth, "filter": must}),
                    );
                }
                json!({"prefetch": branches, "query": {"fusion": "rrf"}, "limit": limit, "with_payload": true})
            }
            _ => {
                let mut body = json!({"query": dense(self)?, "limit": limit, "with_payload": true, "filter": must});
                if hybrid {
                    body["using"] = json!(DENSE);
                }
                body
            }
        };
        let suffix = if matches!(mode, "subjects" | "objects") {
            body["group_by"] = json!(if mode == "subjects" { "subject_key" } else { "object_key" });
            body["group_size"] = json!(1);
            "/groups"
        } else {
            ""
        };
        let response = self.qdrant(
            Method::POST,
            &format!("/collections/{collection}/points/query{suffix}"),
            Some(body),
        )?;
        let points: Vec<&Value> = if suffix.is_empty() {
            response["result"]["points"]
                .as_array()
                .context("invalid Qdrant search results")?
                .iter()
                .collect()
        } else {
            response["result"]["groups"]
                .as_array()
                .context("invalid Qdrant grouped results")?
                .iter()
                .filter_map(|group| group["hits"].as_array().and_then(|hits| hits.first()))
                .collect()
        };
        let mut hits = Vec::new();
        for point in points {
            let entry: Entry = serde_json::from_value(point["payload"]["entry"].clone())?;
            let score = point["score"].as_f64().context("invalid semantic score")?;
            ensure!(score.is_finite(), "non-finite semantic score");
            hits.push(json!({"entry": entry, "score": score}));
        }
        Ok(json!({"index": name, "collection": collection, "hits": hits}))
    }
}

impl Plugin for SemanticSearch {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "semantic-search".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["rebuild".into(), "status".into(), "search".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        let object = input.as_object().context("operation expects a JSON object")?;
        let allowed: &[&str] =
            if operation == "search" { &["index", "text", "limit", "mode"] } else { &["index"] };
        ensure!(
            object.keys().all(|key| allowed.contains(&key.as_str())),
            "unknown semantic operation argument"
        );
        let name = input["index"].as_str().context("index is required")?;
        match operation {
            "rebuild" => self.state.rebuild(context, name),
            "status" => self.state.status(context, name),
            "search" => self.state.search(
                context,
                name,
                input["text"].as_str().context("text is required")?,
                input
                    .get("limit")
                    .map(|value| value.as_u64().context("limit must be a positive integer"))
                    .transpose()?
                    .unwrap_or(10)
                    .try_into()?,
                input
                    .get("mode")
                    .map(|value| value.as_str().context("mode must be a string"))
                    .transpose()?
                    .unwrap_or("matches"),
            ),
            _ => bail!("unknown semantic operation"),
        }
    }
    fn facets(&self) -> Vec<rdf_plugin_api::Facet> {
        self.state
            .config
            .indexes
            .iter()
            .map(|index| rdf_plugin_api::Facet::new("index", index.name.clone()))
            .collect()
    }
    fn property_functions(&self) -> Vec<Arc<dyn PropertyFunction>> {
        ["matches", "subjects", "objects", "lexical", "hybrid"]
            .into_iter()
            .map(|mode| {
                Arc::new(SearchProperty { state: self.state.clone(), mode })
                    as Arc<dyn PropertyFunction>
            })
            .collect()
    }
}
struct SearchProperty {
    state: Arc<State>,
    mode: &'static str,
}
fn string(value: &RdfValue) -> Result<&str> {
    match value {
        RdfValue::Literal { value, datatype, language: None }
            if datatype == &format!("{XSD}string") =>
        {
            Ok(value)
        }
        _ => bail!("expected xsd:string search argument"),
    }
}
impl PropertyFunction for SearchProperty {
    fn descriptor(&self) -> PropertyDescriptor {
        PropertyDescriptor {
            iri: format!("{PREFIX}{}", self.mode),
            description: match self.mode {
                "lexical" => "Lexical (BM25) matches: inputs (index, text, top-k). Keyword score, higher is better. Needs a hybrid-mode index.".into(),
                "hybrid" => "Hybrid matches: inputs (index, text, top-k). Dense and lexical rankings fused with reciprocal rank fusion; the score reflects rank, not similarity. Needs a hybrid-mode index.".into(),
                mode => format!("Semantic {mode}: inputs (index, text, top-k). Cosine score, higher is better. Requires a fresh index."),
            },
            input_arity: 3,
            output_arity: if matches!(self.mode, "subjects" | "objects") { 2 } else { 4 },
        }
    }
    fn evaluate(
        &self,
        context: &dyn ReadContext,
        inputs: &[RdfValue],
        output: &mut dyn RowSink,
    ) -> Result<()> {
        ensure!(inputs.len() == 3, "search expects index, text, and integer top-k");
        let limit = match &inputs[2] {
            RdfValue::Literal { value, datatype, language: None }
                if datatype == &format!("{XSD}integer") =>
            {
                value.parse::<usize>()?
            }
            _ => bail!("top-k must be an xsd:integer"),
        };
        let result = self.state.search(
            context,
            string(&inputs[0])?,
            string(&inputs[1])?,
            limit,
            self.mode,
        )?;
        for hit in result["hits"].as_array().context("missing search hits")? {
            let entry: Entry = serde_json::from_value(hit["entry"].clone())?;
            let score = RdfValue::Literal {
                value: hit["score"].as_f64().context("score")?.to_string(),
                datatype: format!("{XSD}double"),
                language: None,
            };
            let row = match self.mode {
                "subjects" => vec![entry.subject.rdf(), score],
                "objects" => vec![entry.object.rdf(), score],
                _ => vec![entry.subject.rdf(), entry.predicate.rdf(), entry.object.rdf(), score],
            };
            output.emit(row)?;
        }
        Ok(())
    }
}
