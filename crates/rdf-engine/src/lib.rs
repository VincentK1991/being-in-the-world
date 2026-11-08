//! Public database runtime. HTTP and plugin implementations live in separate crates.
use anyhow::{Result, ensure};
use rdf_plugin_api::{CatalogEntry, Manifest, Plugin, ReadContext, SDK_VERSION};
use rdf_store::{engine::Dataset, oxigraph::io::RdfParser};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc};
mod derived;
mod properties;

// RDF/result value types are shared with the adapter for now. No raw Store is exposed.
pub use rdf_store::engine::write_results;
pub use rdf_store::oxigraph::{
    io::RdfFormat,
    model::{GraphName, NamedNode},
    sparql::{QueryResults, results::QueryResultsFormat},
};

#[derive(Clone)]
pub struct Database {
    dataset: Dataset,
    plugins: BTreeMap<String, (Manifest, Arc<dyn Plugin>)>,
    properties: properties::Registry,
    refresh: Vec<RefreshRule>,
}
/// After `after` succeeds, run `then` (for example: rebuild a search index over a graph a job just rewrote).
/// `after.input` matches when every field it lists equals the same field of the operation's input.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRule {
    pub after: OperationRef,
    pub then: OperationRef,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRef {
    pub plugin: String,
    pub operation: String,
    #[serde(default)]
    pub input: Value,
}
#[derive(Serialize)]
pub struct Overview {
    pub quads: usize,
    pub graphs: Vec<String>,
}
#[derive(Serialize)]
pub struct ImportReport {
    pub parsed_quads: u64,
    pub total_quads: usize,
    pub elapsed_ms: u128,
}
pub enum GraphMutation {
    Append,
    Replace,
    Delete,
}

impl Database {
    pub fn open(root: &Path, name: &str, create: bool) -> Result<Self> {
        Ok(Self::from_dataset(Dataset::open(root, name, create)?))
    }
    pub fn memory() -> Result<Self> {
        Ok(Self::from_dataset(Dataset::memory()?))
    }
    pub fn from_dataset(dataset: Dataset) -> Self {
        Self { dataset, plugins: BTreeMap::new(), properties: BTreeMap::new(), refresh: Vec::new() }
    }
    /// Register at composition time, before sharing the database with callers.
    pub fn register(&mut self, plugin: impl Plugin + 'static) -> Result<()> {
        let mut manifest = plugin.manifest();
        let functions = plugin.property_functions();
        let valid = |s: &str| {
            !s.is_empty()
                && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        };
        ensure!(valid(&manifest.id), "invalid plugin ID");
        ensure!(manifest.sdk_version == SDK_VERSION, "incompatible plugin SDK version");
        ensure!(!manifest.version.is_empty(), "missing plugin version");
        ensure!(
            (!manifest.operations.is_empty() || !functions.is_empty())
                && manifest.operations.iter().all(|s| valid(s)),
            "invalid plugin operations"
        );
        let mut unique = manifest.operations.clone();
        unique.sort();
        unique.dedup();
        ensure!(unique.len() == manifest.operations.len(), "duplicate plugin operation");
        ensure!(!self.plugins.contains_key(&manifest.id), "duplicate plugin ID");
        let mut pending = BTreeMap::new();
        for function in functions {
            let descriptor = function.descriptor();
            NamedNode::new(descriptor.iri.clone())?;
            ensure!(
                descriptor.input_arity <= 32 && (1..=32).contains(&descriptor.output_arity),
                "property arity exceeds supported limits"
            );
            ensure!(
                !self.properties.contains_key(&descriptor.iri)
                    && !pending.contains_key(&descriptor.iri),
                "duplicate property IRI: {}",
                descriptor.iri
            );
            pending.insert(descriptor.iri.clone(), (descriptor, function));
        }
        manifest.properties = pending.values().map(|(descriptor, _)| descriptor.clone()).collect();
        self.properties.extend(pending);
        self.plugins.insert(manifest.id.clone(), (manifest, Arc::new(plugin)));
        Ok(())
    }
    /// Everything the registered plugins provide on this dataset, one row per offering.
    pub fn catalog(&self) -> Vec<CatalogEntry> {
        let mut rows = Vec::new();
        for (manifest, plugin) in self.plugins.values() {
            let row = |kind: &str, name: String| CatalogEntry {
                plugin: manifest.id.clone(),
                version: manifest.version.clone(),
                kind: kind.into(),
                name,
            };
            rows.push(row("plugin", manifest.id.clone()));
            rows.extend(manifest.operations.iter().map(|name| row("operation", name.clone())));
            rows.extend(
                manifest.properties.iter().map(|p| row("property-function", p.iri.clone())),
            );
            rows.extend(plugin.facets().into_iter().map(|f| row(&f.kind, f.name)));
        }
        rows
    }
    pub fn plugins(&self) -> Vec<Manifest> {
        self.plugins.values().map(|(m, _)| m.clone()).collect()
    }
    pub fn has_operation(&self, plugin: &str, operation: &str) -> bool {
        self.plugins
            .get(plugin)
            .is_some_and(|(m, _)| m.operations.iter().any(|name| name == operation))
    }
    /// Register at composition time, like plugins. Both operations must exist, and a rule's `then`
    /// may not be another rule's `after`, so one operation never starts a chain.
    pub fn refresh_rules(&mut self, rules: Vec<RefreshRule>) -> Result<()> {
        for rule in &rules {
            ensure!(
                self.has_operation(&rule.after.plugin, &rule.after.operation)
                    && self.has_operation(&rule.then.plugin, &rule.then.operation),
                "refresh rule names an unknown plugin or operation"
            );
            ensure!(
                !rules.iter().any(|other| triggers(
                    &other.after,
                    &rule.then.plugin,
                    &rule.then.operation,
                    &rule.then.input
                )),
                "refresh rules must not chain"
            );
        }
        self.refresh = rules;
        Ok(())
    }
    pub fn invoke(&self, plugin: &str, operation: &str, input: Value) -> Result<Value> {
        let mut result = self.invoke_once(plugin, operation, input.clone())?;
        let ran: Vec<Value> = self
            .refresh
            .iter()
            .filter(|rule| triggers(&rule.after, plugin, operation, &input))
            .map(|rule| {
                let then = &rule.then;
                let outcome = self.invoke_once(&then.plugin, &then.operation, then.input.clone());
                json!({"plugin": then.plugin, "operation": then.operation, "input": then.input,
                    "ok": outcome.is_ok(), "error": outcome.err().map(|e| format!("{e:#}"))})
            })
            .collect();
        if let (false, Some(object)) = (ran.is_empty(), result.as_object_mut()) {
            object.insert("refreshed".into(), Value::Array(ran));
        }
        Ok(result)
    }
    fn invoke_once(&self, plugin: &str, operation: &str, input: Value) -> Result<Value> {
        ensure!(self.has_operation(plugin, operation), "unknown plugin or operation");
        self.plugins[plugin].1.invoke(
            &derived::PluginContext { database: self, owner: plugin },
            operation,
            input,
        )
    }
    pub fn overview(&self) -> Result<Overview> {
        Ok(Overview {
            quads: self.dataset.store().len()?,
            graphs: self
                .dataset
                .store()
                .named_graphs()
                .map(|g| g.map(|g| g.to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
    pub fn query(&self, query: &str) -> Result<QueryResults<'_>> {
        let query = Dataset::prepare_query(query)?;
        let query = properties::prepare(
            query,
            &self.properties,
            &properties::DatasetContext(&self.dataset),
        )?;
        self.dataset.query_prepared(query)
    }
    pub fn update(&self, update: &str) -> Result<()> {
        properties::validate_update(update, &self.properties)?;
        self.dataset.update(update)
    }
    pub fn import(
        &self,
        bytes: &[u8],
        format: RdfFormat,
        graph: Option<NamedNode>,
    ) -> Result<ImportReport> {
        ensure!(!bytes.is_empty(), "The uploaded file is empty.");
        ensure!(
            matches!(
                format,
                RdfFormat::Turtle | RdfFormat::NTriples | RdfFormat::TriG | RdfFormat::NQuads
            ),
            "unsupported RDF format"
        );
        ensure!(
            graph.is_none() || !matches!(format, RdfFormat::TriG | RdfFormat::NQuads),
            "TriG and N-Quads define their own graphs; leave the target graph empty."
        );
        let start = std::time::Instant::now();
        let graph = graph.map(GraphName::NamedNode).unwrap_or(GraphName::DefaultGraph);
        let parser = RdfParser::from_format(format)
            .with_default_graph(graph.clone())
            .rename_blank_nodes()
            .for_reader(bytes);
        let _guard = self.dataset.lock_mutation()?;
        let mut tx = self.dataset.store().start_transaction()?;
        let mut parsed_quads = 0;
        for quad in parser {
            tx.insert(&quad?);
            parsed_quads += 1;
        }
        if let GraphName::NamedNode(name) = &graph {
            tx.insert_named_graph(name);
        }
        tx.commit()?;
        Ok(ImportReport {
            parsed_quads,
            total_quads: self.dataset.store().len()?,
            elapsed_ms: start.elapsed().as_millis(),
        })
    }
    pub fn graph_exists(&self, graph: &GraphName) -> Result<bool> {
        Ok(match graph {
            GraphName::NamedNode(name) => self.dataset.store().contains_named_graph(name)?,
            _ => true,
        })
    }
    pub fn dump_graph(&self, graph: &GraphName, format: RdfFormat) -> Result<Vec<u8>> {
        Ok(self.dataset.store().dump_graph_to_writer(graph, format, Vec::new())?)
    }
    pub fn mutate_graph(
        &self,
        graph: GraphName,
        mutation: GraphMutation,
        format: RdfFormat,
        bytes: &[u8],
    ) -> Result<()> {
        let _guard = self.dataset.lock_mutation()?;
        let mut tx = self.dataset.store().start_transaction()?;
        match mutation {
            GraphMutation::Delete => match &graph {
                GraphName::NamedNode(name) => {
                    tx.remove_named_graph(name)?;
                }
                _ => tx.clear_graph(&graph)?,
            },
            GraphMutation::Append | GraphMutation::Replace => {
                ensure!(
                    matches!(format, RdfFormat::Turtle | RdfFormat::NTriples),
                    "graph input must be Turtle or N-Triples"
                );
                if matches!(mutation, GraphMutation::Replace) {
                    tx.clear_graph(&graph)?;
                }
                if let GraphName::NamedNode(name) = &graph {
                    tx.insert_named_graph(name);
                }
                tx.load_from_reader(
                    RdfParser::from_format(format).with_default_graph(graph).rename_blank_nodes(),
                    bytes,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
impl ReadContext for Database {
    fn query_json(&self, query: &str) -> Result<Value> {
        let results = self.query(query)?;
        ensure!(
            !matches!(results, QueryResults::Graph(_)),
            "plugin JSON query requires SELECT or ASK"
        );
        let mut bytes = Vec::new();
        write_results(results, QueryResultsFormat::Json, RdfFormat::Turtle, &mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// Does `rule` fire for this operation? Its `input` fields must all equal the call's.
fn triggers(rule: &OperationRef, plugin: &str, operation: &str, input: &Value) -> bool {
    rule.plugin == plugin
        && rule.operation == operation
        && rule
            .input
            .as_object()
            .is_none_or(|wanted| wanted.iter().all(|(key, value)| input.get(key) == Some(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdf_plugin_dataset_summary::DatasetSummary;
    struct Incompatible;
    impl Plugin for Incompatible {
        fn manifest(&self) -> Manifest {
            Manifest { sdk_version: SDK_VERSION + 1, ..DatasetSummary.manifest() }
        }
        fn invoke(&self, _: &dyn ReadContext, _: &str, _: Value) -> Result<Value> {
            anyhow::bail!("incompatible plugins must never execute")
        }
    }
    #[test]
    fn rejects_incompatible_plugin_before_registration() -> Result<()> {
        let mut database = Database::memory()?;
        assert!(database.register(Incompatible).is_err());
        assert!(database.plugins().is_empty());
        Ok(())
    }

    #[test]
    fn import_and_graph_replacement_roll_back_without_http() -> Result<()> {
        let database = Database::memory()?;
        let graph = NamedNode::new("urn:graph")?;
        database.import(b"<urn:s> <urn:p> 1 .", RdfFormat::Turtle, Some(graph.clone()))?;
        assert!(
            database
                .import(b"<urn:new> <urn:p> 2 . invalid", RdfFormat::Turtle, Some(graph.clone()))
                .is_err()
        );
        assert!(
            database
                .mutate_graph(
                    GraphName::NamedNode(graph.clone()),
                    GraphMutation::Replace,
                    RdfFormat::Turtle,
                    b"invalid"
                )
                .is_err()
        );
        assert_eq!(database.overview()?.quads, 1);
        assert_eq!(
            database.query_json("ASK { GRAPH <urn:graph> { <urn:s> <urn:p> 1 } }")?["boolean"],
            true
        );
        Ok(())
    }
    #[test]
    fn plugin_is_dataset_scoped_and_works_without_http() -> Result<()> {
        let mut a = Database::memory()?;
        a.register(DatasetSummary)?;
        assert!(a.register(DatasetSummary).is_err());
        a.update("INSERT DATA { <urn:s> <urn:p> <urn:o> }")?;
        let mut b = Database::memory()?;
        b.register(DatasetSummary)?;
        let count = |db: &Database| -> Result<Value> {
            Ok(db.invoke("dataset-summary", "count", serde_json::json!({}))?["results"]["bindings"]
                [0]["quads"]["value"]
                .clone())
        };
        assert_eq!(count(&a)?, "1");
        assert_eq!(count(&b)?, "0");
        assert!(a.invoke("missing", "count", Value::Null).is_err());
        assert!(a.invoke("dataset-summary", "missing", Value::Null).is_err());
        assert!(a.invoke("dataset-summary", "count", Value::Null).is_err());
        assert!(
            a.query_json("SELECT * WHERE { SERVICE <https://example.org/> { ?s ?p ?o } }").is_err()
        );
        Ok(())
    }
}
