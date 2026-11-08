//! Lets a query ask a dataset which plugins it has. Federated queries can ask a remote dataset
//! the same way, inside `SERVICE`:
//!
//! ```sparql
//! (?plugin ?kind ?name) <urn:rdftool:plugins:provides> ()
//! (?plugin ?version)    <urn:rdftool:plugins:available> ()
//! ```
//!
//! The catalog is a snapshot taken after every other plugin is registered (plugin configuration
//! is read at startup, so it cannot change while the server runs). Live state such as a stale
//! inference job stays with each plugin's `status` operation.
use anyhow::{Result, ensure};
use rdf_plugin_api::{
    CatalogEntry, Manifest, Plugin, PropertyDescriptor, PropertyFunction, RdfValue, ReadContext,
    RowSink, SDK_VERSION,
};
use serde_json::Value;
use std::sync::Arc;

pub const PREFIX: &str = "urn:rdftool:plugins:";

pub struct PluginCatalog {
    rows: Arc<Vec<CatalogEntry>>,
}
impl PluginCatalog {
    /// `others` is `Database::catalog()` taken before this plugin is registered.
    pub fn new(mut rows: Vec<CatalogEntry>) -> Self {
        let version = env!("CARGO_PKG_VERSION");
        let own = |kind: &str, name: &str| CatalogEntry {
            plugin: "plugin-catalog".into(),
            version: version.into(),
            kind: kind.into(),
            name: name.into(),
        };
        rows.push(own("plugin", "plugin-catalog"));
        for name in ["available", "provides"] {
            rows.push(own("property-function", &format!("{PREFIX}{name}")));
        }
        Self { rows: Arc::new(rows) }
    }
}
impl Plugin for PluginCatalog {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "plugin-catalog".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec![],
            properties: vec![],
        }
    }
    fn invoke(&self, _context: &dyn ReadContext, _operation: &str, _input: Value) -> Result<Value> {
        anyhow::bail!("plugin-catalog has no operations; use its property functions")
    }
    fn property_functions(&self) -> Vec<Arc<dyn PropertyFunction>> {
        vec![
            Arc::new(Catalog { rows: self.rows.clone(), provides: false }),
            Arc::new(Catalog { rows: self.rows.clone(), provides: true }),
        ]
    }
}
struct Catalog {
    rows: Arc<Vec<CatalogEntry>>,
    provides: bool,
}
impl PropertyFunction for Catalog {
    fn descriptor(&self) -> PropertyDescriptor {
        if self.provides {
            PropertyDescriptor {
                iri: format!("{PREFIX}provides"),
                description: "Everything the dataset's plugins offer. No inputs; outputs (plugin, kind, name). Kinds: operation, property-function, index, job, virtual-graph.".into(),
                input_arity: 0,
                output_arity: 3,
            }
        } else {
            PropertyDescriptor {
                iri: format!("{PREFIX}available"),
                description: "The plugins this dataset has. No inputs; outputs (plugin, version)."
                    .into(),
                input_arity: 0,
                output_arity: 2,
            }
        }
    }
    fn evaluate(
        &self,
        _context: &dyn ReadContext,
        inputs: &[RdfValue],
        output: &mut dyn RowSink,
    ) -> Result<()> {
        ensure!(inputs.is_empty(), "takes no inputs; write the object as ()");
        for row in self.rows.iter() {
            if self.provides {
                output.emit(vec![
                    RdfValue::string(&row.plugin),
                    RdfValue::string(&row.kind),
                    RdfValue::string(&row.name),
                ])?;
            } else if row.kind == "plugin" {
                output.emit(vec![RdfValue::string(&row.plugin), RdfValue::string(&row.version)])?;
            }
        }
        Ok(())
    }
}
