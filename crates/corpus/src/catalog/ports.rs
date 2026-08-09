//! D5 Corpus catalog/executor split (Agent Kernel V2).
//!
//! The rich tool catalog stays in Corpus; platform/provider/process move to
//! adapters; the Kernel registry is bootstrap-only and seals before serving.
//! This seam defines the catalog port and the executor port that separate the
//! catalog data from the execution path.  The Kernel `CapabilityRegistry`
//! (K2) provides the sealed bootstrap-only registry.  No writer cutover.

use async_trait::async_trait;

/// A rich catalog entry.  This stays in Corpus (owner of the catalog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    /// Payload is the rich catalog definition; it never carries secrets.
    pub definition: serde_json::Value,
}

/// Catalog port — reads the rich catalog.  Corpus owns this.
#[async_trait]
pub trait CatalogPort: Send + Sync {
    async fn list(&self) -> Vec<CatalogEntry>;
}

/// Executor port — the execution boundary, separated from the catalog.
/// Platform/provider/process adapters implement this; the Kernel sealed
/// registry resolves the executor binding (bootstrap-only, sealed before
/// serving).
#[async_trait]
pub trait ExecutorPort: Send + Sync {
    async fn execute(
        &self,
        entry_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ExecutorError>;
}

/// Typed executor errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutorError {
    #[error("entry not found in catalog")]
    EntryNotFound,
    #[error("executor unavailable")]
    ExecutorUnavailable,
}

/// In-memory catalog + executor for testing the split.
#[derive(Default)]
pub struct InMemoryCatalog {
    entries: Vec<CatalogEntry>,
}

impl InMemoryCatalog {
    pub fn new(entries: Vec<CatalogEntry>) -> Self {
        Self { entries }
    }
}

#[async_trait]
impl CatalogPort for InMemoryCatalog {
    async fn list(&self) -> Vec<CatalogEntry> {
        self.entries.clone()
    }
}

#[derive(Default)]
pub struct EchoExecutor;

#[async_trait]
impl ExecutorPort for EchoExecutor {
    async fn execute(
        &self,
        entry_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ExecutorError> {
        Ok(serde_json::json!({ "entry": entry_id, "echo": input }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_and_executor_are_separate_ports() {
        let catalog = InMemoryCatalog::new(vec![CatalogEntry {
            id: "tool.x".into(),
            name: "x".into(),
            definition: serde_json::json!({"kind": "shell"}),
        }]);
        let executor = EchoExecutor;

        let entries = catalog.list().await;
        assert_eq!(entries.len(), 1);

        let out = executor
            .execute("tool.x", serde_json::json!({"a": 1}))
            .await
            .unwrap();
        assert_eq!(out["echo"]["a"], 1);
    }
}
