//! Infrastructure configuration: Sandbox, McpServer, Plugins, Memory, Daemon.

use serde::{Deserialize, Serialize};

/// Sandbox execution preference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// "auto", "require", or "forbid"
    #[serde(default = "default_sandbox_preference")]
    pub preference: String,
    #[serde(default)]
    pub bubblewrap_path: Option<String>,
}

pub(crate) fn default_sandbox_preference() -> String {
    "auto".to_string()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            preference: default_sandbox_preference(),
            bubblewrap_path: None,
        }
    }
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// "stdio", "http", or "sse"
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// For stdio transport: command to run
    #[serde(default)]
    pub command: Option<String>,
    /// For http/sse transport: URL to connect to
    #[serde(default)]
    pub url: Option<String>,
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: default_mcp_transport(),
            command: None,
            url: None,
        }
    }
}

/// Plugin directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default)]
    pub directories: Vec<String>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            directories: Vec::new(),
        }
    }
}

/// Memory backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// "sqlite" or "in_memory"
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    #[serde(default = "default_memory_data_dir")]
    pub data_dir: String,
}

pub(crate) fn default_memory_backend() -> String {
    "sqlite".to_string()
}
pub(crate) fn default_memory_data_dir() -> String {
    "~/.aletheon/memory".to_string()
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
            data_dir: default_memory_data_dir(),
        }
    }
}

/// Daemon runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_socket_path")]
    pub socket_path: String,
    #[serde(default = "default_daemon_log_level")]
    pub log_level: String,
}

pub(crate) fn default_daemon_socket_path() -> String {
    "/run/aletheond/aletheond.sock".to_string()
}
pub(crate) fn default_daemon_log_level() -> String {
    "info".to_string()
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: default_daemon_socket_path(),
            log_level: default_daemon_log_level(),
        }
    }
}
