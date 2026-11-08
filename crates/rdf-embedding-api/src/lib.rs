//! Provider extension contract. No database or HTTP dependencies.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// Identity of the vector space, excluding credentials and execution device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub identity: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub profile: Profile,
    pub device: String,
}
pub trait EmbeddingProvider: Send + Sync {
    fn health(&self) -> Result<Health>;
    /// Must preserve input order and check that the effective model matches expected.
    fn embed(&self, texts: &[String], expected: &Profile) -> Result<Vec<Vec<f32>>>;
}
pub trait ProviderFactory: Send + Sync {
    fn create(&self, config: &Value) -> Result<Arc<dyn EmbeddingProvider>>;
}
#[derive(Default)]
pub struct ProviderRegistry {
    factories: BTreeMap<String, Arc<dyn ProviderFactory>>,
}
impl ProviderRegistry {
    pub fn register(&mut self, kind: &str, factory: impl ProviderFactory + 'static) -> Result<()> {
        ensure!(
            !kind.is_empty() && !self.factories.contains_key(kind),
            "duplicate or empty provider kind"
        );
        self.factories.insert(kind.into(), Arc::new(factory));
        Ok(())
    }
    pub fn create(&self, kind: &str, config: &Value) -> Result<Arc<dyn EmbeddingProvider>> {
        self.factories
            .get(kind)
            .context("embedding provider kind is not registered")?
            .create(config)
    }
}
pub fn validate(vectors: &[Vec<f32>], count: usize, profile: &Profile) -> Result<()> {
    ensure!((1..=65536).contains(&profile.dimension), "invalid embedding dimension");
    ensure!(vectors.len() == count, "embedding batch length mismatch");
    for v in vectors {
        ensure!(
            v.len() == profile.dimension
                && v.iter().all(|x| x.is_finite())
                && v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() > 0.0,
            "invalid embedding vector"
        );
    }
    Ok(())
}
