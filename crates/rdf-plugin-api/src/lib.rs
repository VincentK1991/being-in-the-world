//! SDK v3: trusted operations, property functions and host-managed derived graphs.
//! No dependency on HTTP, Oxigraph, or filesystem storage.
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

pub const SDK_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub sdk_version: u32,
    pub operations: Vec<String>,
    /// Filled by the host from the plugin's registered property implementations.
    pub properties: Vec<PropertyDescriptor>,
}

/// Dataset-scoped access. Query policy remains enforced by the host.
/// Each call has its own engine snapshot; multiple calls are not one transaction.
pub trait ReadContext {
    fn query_snapshot_json(&self, _snapshot: &GraphSnapshot, _query: &str) -> Result<Value> {
        anyhow::bail!("snapshot query unavailable")
    }
    /// Conservative local-dataset guard. Not a remote snapshot token.
    fn publication_token(&self) -> Result<String> {
        anyhow::bail!("publication guards unavailable")
    }
    fn publish_checked(
        &self,
        name: &str,
        source: &GraphSnapshot,
        ntriples: &str,
        metadata: Value,
        token: Option<&str>,
    ) -> Result<Value> {
        anyhow::ensure!(token.is_none(), "publication guards unavailable");
        self.publish(name, source, ntriples, metadata)
    }

    fn query_json(&self, sparql: &str) -> Result<Value>;
    fn snapshot(&self, _graphs: &[String]) -> Result<GraphSnapshot> {
        anyhow::bail!("snapshot capability unavailable")
    }
    fn publish(
        &self,
        _name: &str,
        _source: &GraphSnapshot,
        _ntriples: &str,
        _metadata: Value,
    ) -> Result<Value> {
        anyhow::bail!("derived graph publication unavailable")
    }
    fn derived_status(&self, _name: &str) -> Result<Value> {
        anyhow::bail!("derived graph status unavailable")
    }
    /// Keep a note about the last attempt that did not publish (an inconsistent input, say) next
    /// to the derived graph's record, or clear it with `None`. `derived_status` returns it as
    /// `attempt`; a later `publish` clears it. The derived graph itself is never touched.
    fn record_attempt(&self, _name: &str, _attempt: Option<Value>) -> Result<()> {
        anyhow::bail!("attempt records unavailable")
    }
}

/// Implementations must be thread-safe. Native plugins are trusted application code.
/// An operation must not call the host HTTP API or open database files itself.
/// Explicit external provider calls are allowed; publication goes through the host.
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> Manifest;
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value>;
    fn property_functions(&self) -> Vec<std::sync::Arc<dyn PropertyFunction>> {
        Vec::new()
    }
    /// Configured things this plugin offers on this dataset (indexes, jobs, ...), for discovery.
    fn facets(&self) -> Vec<Facet> {
        Vec::new()
    }
}

/// One configured offering, e.g. kind `index` and name `descriptions`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Facet {
    pub kind: String,
    pub name: String,
}
impl Facet {
    pub fn new(kind: &str, name: impl Into<String>) -> Self {
        Self { kind: kind.into(), name: name.into() }
    }
}

/// One row of a dataset's plugin catalog: a plugin and one thing it provides.
/// Kinds: `operation`, `property-function`, plus whatever a plugin declares as a facet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    pub plugin: String,
    pub version: String,
    pub kind: String,
    pub name: String,
}

/// SDK-owned RDF 1.1 values. Blank nodes are excluded from property tables in v2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RdfValue {
    Iri(String),
    Literal { value: String, datatype: String, language: Option<String> },
}
impl RdfValue {
    pub fn string(value: impl Into<String>) -> Self {
        Self::Literal {
            value: value.into(),
            datatype: "http://www.w3.org/2001/XMLSchema#string".into(),
            language: None,
        }
    }
    pub fn integer(value: usize) -> Self {
        Self::Literal {
            value: value.to_string(),
            datatype: "http://www.w3.org/2001/XMLSchema#integer".into(),
            language: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PropertyDescriptor {
    pub iri: String,
    pub description: String,
    pub input_arity: usize,
    pub output_arity: usize,
}

/// Emit rows incrementally. Propagate errors; the host enforces total row/byte limits.
pub trait RowSink {
    fn emit(&mut self, row: Vec<RdfValue>) -> Result<()>;
}

/// Pure/read-only relation invoked once per syntactic occurrence during preparation.
/// Inputs are constants, outputs bind the subject (or subject list) of a triple.
/// Context reads use the selected dataset, with one snapshot per context call.
/// Native implementations remain trusted, unsandboxed code.
pub trait PropertyFunction: Send + Sync {
    fn descriptor(&self) -> PropertyDescriptor;
    fn evaluate(
        &self,
        context: &dyn ReadContext,
        inputs: &[RdfValue],
        output: &mut dyn RowSink,
    ) -> Result<()>;
}

/// One selected-graph snapshot; blank-node labels are retained across the exchange.
#[derive(Clone, Debug, Serialize)]
pub struct GraphSnapshot {
    pub graphs: Vec<String>,
    pub token: String,
    pub ntriples: String,
}
