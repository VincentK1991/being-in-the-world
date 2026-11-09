//! What a tool needs to know about a dataset before it can help an agent: its graphs, plugins,
//! property functions, inference jobs and federation allowlist. Fetched together and cached for a
//! few seconds, so a burst of tool calls costs one round of requests.
use crate::client::Gateway;
use crate::render::{GraphKind, bindings, plain};
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct Graph {
    pub iri: String,
    pub kind: GraphKind,
    pub triples: u64,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub iri: String,
    pub plugin: String,
    pub description: String,
    pub inputs: usize,
    pub outputs: usize,
}

#[derive(Debug, Default)]
pub struct DatasetContext {
    pub graphs: Vec<Graph>,
    pub default_triples: u64,
    pub plugins: Vec<Value>,
    pub functions: BTreeMap<String, Function>,
    /// (plugin, kind, name) rows from the catalog, operations excluded.
    pub facets: Vec<(String, String, String)>,
    /// Inference job name -> status summary (`state`, `triples`, ...).
    pub jobs: BTreeMap<String, Value>,
    pub federation: Option<Value>,
}

impl DatasetContext {
    pub fn has_plugin(&self, id: &str) -> bool {
        self.plugins.iter().any(|p| p["id"] == id)
    }
    pub fn graphs_of(&self, wanted: impl Fn(&GraphKind) -> bool) -> Vec<String> {
        self.graphs.iter().filter(|g| wanted(&g.kind)).map(|g| g.iri.clone()).collect()
    }
    /// Asserted plus inferred graphs: the data an agent normally means.
    pub fn data_graphs(&self) -> Vec<String> {
        self.graphs_of(|k| matches!(k, GraphKind::Asserted | GraphKind::Inferred(_)))
    }
    pub fn graph_names(&self) -> BTreeSet<String> {
        self.graphs.iter().map(|g| g.iri.clone()).collect()
    }
    pub fn facets(&self, kind: &str) -> Vec<String> {
        self.facets.iter().filter(|(_, k, _)| k == kind).map(|(_, _, n)| n.clone()).collect()
    }
    pub fn allowlist(&self) -> Vec<String> {
        self.federation
            .as_ref()
            .and_then(|f| f["allow"].as_array())
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }
    /// Freshness notes for the inferred graphs in `graphs`.
    pub fn freshness(&self, graphs: &[String]) -> Vec<Value> {
        let mut notes = Vec::new();
        for g in graphs {
            if let GraphKind::Inferred(job) = GraphKind::of(g)
                && let Some(status) = self.jobs.get(&job)
            {
                notes.push(json!({"job": job, "state": status["state"]}));
            }
        }
        notes
    }
}

type Cache = HashMap<String, (Instant, Arc<DatasetContext>)>;

#[derive(Clone)]
pub struct Contexts {
    gateway: Gateway,
    cache: Arc<Mutex<Cache>>,
}

impl Contexts {
    pub fn new(gateway: Gateway) -> Self {
        Self { gateway, cache: Arc::default() }
    }

    pub async fn get(&self, dataset: &str) -> Result<Arc<DatasetContext>> {
        if let Some((at, context)) = self.cache.lock().await.get(dataset)
            && at.elapsed() < TTL
        {
            return Ok(context.clone());
        }
        let context = Arc::new(self.load(dataset).await?);
        self.cache.lock().await.insert(dataset.to_owned(), (Instant::now(), context.clone()));
        Ok(context)
    }

    async fn load(&self, dataset: &str) -> Result<DatasetContext> {
        let gw = &self.gateway;
        let mut context = DatasetContext::default();

        let counts = gw
            .select(
                dataset,
                "SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g",
            )
            .await?;
        for row in bindings(&counts) {
            if let Some(g) = plain(row, "g") {
                let triples = plain(row, "n").and_then(|n| n.parse().ok()).unwrap_or(0);
                context.graphs.push(Graph { iri: g.to_owned(), kind: GraphKind::of(g), triples });
            }
        }
        context.graphs.sort_by(|a, b| a.iri.cmp(&b.iri));
        let default = gw.select(dataset, "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }").await?;
        context.default_triples = bindings(&default)
            .first()
            .and_then(|r| plain(r, "n"))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);

        context.plugins = gw.plugins(dataset).await?.as_array().cloned().unwrap_or_default();
        for plugin in &context.plugins {
            let id = plugin["id"].as_str().unwrap_or_default();
            for p in plugin["properties"].as_array().into_iter().flatten() {
                let iri = p["iri"].as_str().unwrap_or_default().to_owned();
                context.functions.insert(
                    iri.clone(),
                    Function {
                        iri,
                        plugin: id.to_owned(),
                        description: p["description"].as_str().unwrap_or_default().to_owned(),
                        inputs: p["input_arity"].as_u64().unwrap_or(0) as usize,
                        outputs: p["output_arity"].as_u64().unwrap_or(0) as usize,
                    },
                );
            }
        }
        if context.functions.contains_key("urn:rdftool:plugins:provides")
            && let Ok(rows) = gw
                .select(
                    dataset,
                    "SELECT ?p ?k ?n WHERE { (?p ?k ?n) <urn:rdftool:plugins:provides> () }",
                )
                .await
        {
            for row in bindings(&rows) {
                if let (Some(p), Some(k), Some(n)) =
                    (plain(row, "p"), plain(row, "k"), plain(row, "n"))
                    && k != "operation"
                    && k != "plugin"
                    && k != "property-function"
                {
                    context.facets.push((p.to_owned(), k.to_owned(), n.to_owned()));
                }
            }
        }
        if context.has_plugin("inference")
            && let Ok(status) =
                gw.operation(dataset, "inference", "status", json!({"inputs": false})).await
        {
            for job in status["jobs"].as_array().into_iter().flatten() {
                if let Some(name) = job["job"].as_str() {
                    context.jobs.insert(
                        name.to_owned(),
                        json!({
                            "state": job["state"], "triples": job["triples"], "graph": job["graph"],
                            "sources": job["graphs"], "depends_on": job["depends_on"],
                            "profile": job["metadata"]["profile"], "clashes": job["clashes"],
                            "consistency": job["metadata"]["consistency"],
                        }),
                    );
                }
            }
        }
        if context.has_plugin("federation") {
            context.federation =
                gw.operation(dataset, "federation", "status", json!({})).await.ok();
        }
        Ok(context)
    }
}
