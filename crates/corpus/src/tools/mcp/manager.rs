//! High-level MCP facade for the daemon: connect configured servers and expose
//! their tools as `Box<dyn Tool>` ready to register into the ToolRegistry.

use anyhow::Result;

use super::client::McpConnectionManager;
use super::config::McpConfig;
use crate::tools::Tool;

/// Thin facade that owns an [`McpConnectionManager`] and exposes a
/// register-friendly interface.
pub struct McpManager {
    inner: McpConnectionManager,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            inner: McpConnectionManager::new(config),
        }
    }

    /// Connect to every enabled server in the config.
    pub async fn connect_all(&mut self) -> Result<()> {
        self.inner.connect_all().await
    }

    /// Return discovered tools as boxed [`Tool`] trait objects, ready to be
    /// inserted into a `ToolRegistry`.
    pub fn tool_wrappers(&self) -> Vec<Box<dyn Tool>> {
        self.inner
            .get_all_tools()
            .into_iter()
            .map(|w| w.boxed_clone())
            .collect()
    }

    /// Number of servers that were successfully connected.
    pub fn connected_count(&self) -> usize {
        self.inner.connected_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_config_connect_all_ok() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        let result = mgr.connect_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn empty_config_tool_wrappers_empty() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert!(mgr.tool_wrappers().is_empty());
    }

    #[tokio::test]
    async fn empty_config_connected_count_zero() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 0);
    }
}
