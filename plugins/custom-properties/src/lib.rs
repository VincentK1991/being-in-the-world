//! Extensible property collection. Add implementations without editing engine/server.
use anyhow::{Result, bail, ensure};
use rdf_plugin_api::{
    Manifest, Plugin, PropertyDescriptor, PropertyFunction, RdfValue, ReadContext, RowSink,
    SDK_VERSION,
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Default)]
pub struct CustomProperties {
    functions: Vec<Arc<dyn PropertyFunction>>,
}
impl CustomProperties {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_property(mut self, function: impl PropertyFunction + 'static) -> Self {
        self.functions.push(Arc::new(function));
        self
    }
    pub fn examples() -> Self {
        Self::new().with_property(Split).with_property(Tokens)
    }
}
impl Plugin for CustomProperties {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "custom-properties".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            sdk_version: SDK_VERSION,
            operations: vec![],
            properties: vec![],
        }
    }
    fn invoke(&self, _: &dyn ReadContext, _: &str, _: Value) -> Result<Value> {
        bail!("use property predicates in SPARQL queries")
    }
    fn property_functions(&self) -> Vec<Arc<dyn PropertyFunction>> {
        self.functions.clone()
    }
}
fn string(value: &RdfValue) -> Result<&str> {
    match value {
        RdfValue::Literal { value, datatype, language: None }
            if datatype == "http://www.w3.org/2001/XMLSchema#string" =>
        {
            Ok(value)
        }
        _ => bail!("expected an xsd:string literal"),
    }
}
/// ?part <urn:rdftool:property:split> ("a,b,a" ",")
pub struct Split;
impl PropertyFunction for Split {
    fn descriptor(&self) -> PropertyDescriptor {
        PropertyDescriptor { iri: "urn:rdftool:property:split".into(), description: "Split a string by a nonempty literal separator; preserve order, duplicates and empty fields.".into(), input_arity: 2, output_arity: 1 }
    }
    fn evaluate(
        &self,
        _: &dyn ReadContext,
        inputs: &[RdfValue],
        output: &mut dyn RowSink,
    ) -> Result<()> {
        ensure!(inputs.len() == 2, "split expects two inputs");
        let value = string(&inputs[0])?;
        let separator = string(&inputs[1])?;
        ensure!(!separator.is_empty(), "split separator must not be empty");
        for part in value.split(separator) {
            output.emit(vec![RdfValue::string(part)])?;
        }
        Ok(())
    }
}
/// (?token ?position) <urn:rdftool:property:tokens> "hello world"
pub struct Tokens;
impl PropertyFunction for Tokens {
    fn descriptor(&self) -> PropertyDescriptor {
        PropertyDescriptor { iri: "urn:rdftool:property:tokens".into(), description: "Split on Unicode whitespace; emit token and zero-based position. Demonstration only, not a search index.".into(), input_arity: 1, output_arity: 2 }
    }
    fn evaluate(
        &self,
        _: &dyn ReadContext,
        inputs: &[RdfValue],
        output: &mut dyn RowSink,
    ) -> Result<()> {
        ensure!(inputs.len() == 1, "tokens expects one input");
        for (position, token) in string(&inputs[0])?.split_whitespace().enumerate() {
            output.emit(vec![RdfValue::string(token), RdfValue::integer(position)])?;
        }
        Ok(())
    }
}
