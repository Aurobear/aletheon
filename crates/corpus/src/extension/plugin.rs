//! Long-lived extension plugin lifecycle contract.
//!
//! Execute-only tools do not need this contract. Extension hosts use it when a
//! package needs explicit initialization, background work, and shutdown.

use std::sync::Arc;

use ::contracts::{Tool, Version};
use anyhow::Result;
use async_trait::async_trait;

/// Context handed to an extension plugin during initialization.
pub struct PluginContext {
    pub plugin_id: String,
    pub working_dir: std::path::PathBuf,
    pub config: serde_json::Value,
}

/// Lifecycle implemented by a long-lived extension plugin.
#[async_trait]
pub trait Plugin: Send + Sync {
    fn id(&self) -> &str;

    fn version(&self) -> Version;

    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;

    async fn run(&mut self) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()>;

    fn capabilities(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SamplePlugin {
        init_calls: Arc<AtomicUsize>,
        shutdown_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Plugin for SamplePlugin {
        fn id(&self) -> &str {
            "sample"
        }

        fn version(&self) -> Version {
            Version::new(0, 1, 0)
        }

        async fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
            self.init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn defaults_and_lifecycle_hooks_work() {
        let init_calls = Arc::new(AtomicUsize::new(0));
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let mut plugin = SamplePlugin {
            init_calls: Arc::clone(&init_calls),
            shutdown_calls: Arc::clone(&shutdown_calls),
        };
        let context = PluginContext {
            plugin_id: "sample".into(),
            working_dir: std::path::PathBuf::from("."),
            config: serde_json::Value::Null,
        };

        plugin.run().await.unwrap();
        assert!(plugin.capabilities().is_empty());
        plugin.init(&context).await.unwrap();
        plugin.shutdown().await.unwrap();

        assert_eq!(init_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
        assert_eq!(plugin.id(), "sample");
    }
}
