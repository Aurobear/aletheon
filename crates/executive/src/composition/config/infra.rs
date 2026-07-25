//! Infrastructure configuration: Sandbox, McpServer, Plugins, Memory, Daemon.
//!
//! All types are re-exported from aletheon-cognit to avoid duplication.

pub use cognit::config::DaemonConfig;
pub use cognit::config::PluginsConfig;
pub use cognit::config::SandboxConfig;
pub use corpus::tools::mcp::config::McpServerConfig;
