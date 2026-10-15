//! Bundle-specific strategies. The shared card consumes a neutral projection.
mod environment;
mod vinted;
use super::AppInstance;
pub use environment::EnvironmentStrategy;
use std::collections::HashMap;
pub use vinted::VintedStrategy;

#[derive(Debug, Clone)]
pub struct CardFact {
    pub label: String,
    pub value: String,
}
#[derive(Debug, Clone)]
pub struct CardDetails {
    pub app_name: String,
    pub symbol: &'static str,
    pub accent: u32,
    pub facts: Vec<CardFact>,
    pub action_label: &'static str,
}
pub trait AppStrategy: Send + Sync {
    fn project(&self, app: &AppInstance, backup_count: usize) -> CardDetails;
}
pub struct StrategyRegistry {
    plugins: HashMap<String, Box<dyn AppStrategy>>,
    fallback: Box<dyn AppStrategy>,
}
impl Default for StrategyRegistry {
    fn default() -> Self {
        let mut registry = Self::new(Box::new(EnvironmentStrategy));
        registry.register("lt.manodrabuziai.fr", Box::new(VintedStrategy));
        registry
    }
}
impl StrategyRegistry {
    pub fn new(fallback: Box<dyn AppStrategy>) -> Self {
        Self {
            plugins: HashMap::new(),
            fallback,
        }
    }
    pub fn register(&mut self, bundle: impl Into<String>, strategy: Box<dyn AppStrategy>) {
        self.plugins.insert(bundle.into(), strategy);
    }
    pub fn for_bundle(&self, bundle: &str) -> &dyn AppStrategy {
        self.plugins
            .get(bundle)
            .map(|v| v.as_ref())
            .unwrap_or(self.fallback.as_ref())
    }
}
