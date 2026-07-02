//! Plugin lifecycle contract -- the long-lived counterpart to execute-only tools.
//!
//! A plugin MAY implement this trait for `init` / `run` / `shutdown` behavior and
//! to register additional capabilities (tools). Plugins that only expose
//! execute-only `Tool`s do not need to implement it -- the trait is additive.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::include::subsystem::Version;
use crate::types::tool::Tool;

/// Context handed to a plugin at `init`.
///
/// Kept intentionally small: the plugin's id, the directory its manifest lives
/// in, and its parsed configuration (from the manifest / host).
pub struct PluginContext {
    pub plugin_id: String,
    pub working_dir: std::path::PathBuf,
    pub config: serde_json::Value,
}

/// Long-lived plugin lifecycle -- the seam for `init` / `run` / `shutdown`.
///
/// The host (`PluginManager`) calls `init` on load and `shutdown` on unload,
/// tracked by the existing `PluginState`. `run` is an optional long-lived hook
/// that defaults to a no-op.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Stable plugin identifier (matches the manifest `id`).
    fn id(&self) -> &str;

    /// Plugin version, for ABI/compatibility checks.
    fn version(&self) -> Version;

    /// Called once when the plugin is loaded. Set up resources here.
    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;

    /// Optional long-lived behavior. Defaults to a no-op so `Tool`-only and
    /// short-lived plugins need not implement it.
    async fn run(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called once when the plugin is unloaded. Flush and release resources here.
    async fn shutdown(&mut self) -> Result<()>;

    /// Additional capabilities (tools) this plugin registers. Defaults to none;
    /// the host merges these into the plugin's execute-only tool set.
    fn capabilities(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    struct SamplePlugin {
        init_calls: StdArc<AtomicUsize>,
        shutdown_calls: StdArc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Plugin for SamplePlugin {
        fn id(&self) -> &str {
            "sample"
        }
        fn version(&self) -> crate::include::subsystem::Version {
            crate::include::subsystem::Version::new(0, 1, 0)
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
    async fn plugin_default_methods_and_hooks() {
        let init = StdArc::new(AtomicUsize::new(0));
        let down = StdArc::new(AtomicUsize::new(0));
        let mut p = SamplePlugin {
            init_calls: init.clone(),
            shutdown_calls: down.clone(),
        };
        let ctx = PluginContext {
            plugin_id: "sample".into(),
            working_dir: std::path::PathBuf::from("."),
            config: serde_json::Value::Null,
        };
        // default run() is a no-op; capabilities() defaults empty
        assert!(p.run().await.is_ok());
        assert!(p.capabilities().is_empty());
        p.init(&ctx).await.unwrap();
        p.shutdown().await.unwrap();
        assert_eq!(init.load(Ordering::SeqCst), 1);
        assert_eq!(down.load(Ordering::SeqCst), 1);
        assert_eq!(p.id(), "sample");
    }
}
