//! Example independently developed plugin; depends only on the SDK.
use anyhow::{Result, ensure};
use rdf_plugin_api::{Manifest, Plugin, ReadContext, SDK_VERSION};
use serde_json::Value;

pub struct DatasetSummary;
impl Plugin for DatasetSummary {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "dataset-summary".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec!["count".into()],
            properties: vec![],
        }
    }
    fn invoke(&self, context: &dyn ReadContext, operation: &str, input: Value) -> Result<Value> {
        ensure!(operation == "count", "unsupported operation");
        ensure!(
            input.as_object().is_some_and(|object| object.is_empty()),
            "count expects an empty JSON object"
        );
        context.query_json(
            "SELECT (COUNT(*) AS ?quads) WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }",
        )
    }
}
