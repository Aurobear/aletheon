use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::tools::PermissionLevel;

/// Default per-request timeout in milliseconds (30 seconds).
pub fn default_request_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
    pub tool_name_prefix: bool,
    pub max_tool_name_length: usize,
    /// Tool names that are explicitly allowed. If non-empty, only these tools
    /// are registered — all others from MCP servers are silently skipped.
    /// Empty = allow all (default for backward compatibility).
    /// Supports prefix matching: "mcp.github." matches "mcp.github.list_repos".
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Tool names that are explicitly denied. Takes precedence over allowlist.
    /// Denied tools are silently skipped during registration.
    /// Supports prefix matching.
    #[serde(default)]
    pub tool_denylist: Vec<String>,
    /// Per-tool permission overrides. The preferred key is the final registered
    /// tool name (for example `github__delete_repo` or
    /// `mcp.github.resource.readme`). Legacy server-name keys remain accepted as
    /// a lower-precedence fallback.
    #[serde(default)]
    pub permission_overrides: HashMap<String, PermissionLevel>,
    /// Global default per-request timeout in milliseconds.
    /// Individual servers can override via [`McpServerConfig::request_timeout_ms`].
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            tool_name_prefix: true,
            max_tool_name_length: 64,
            tool_allowlist: Vec::new(),
            tool_denylist: Vec::new(),
            permission_overrides: HashMap::new(),
            request_timeout_ms: default_request_timeout_ms(),
        }
    }
}

// Wave 2B: explicit type-path imports replace blanket re-export.
// Full dependency inversion (moving ownership out of cognit) requires
// breaking the circular dependency cognit ↔ corpus via a shared crate.
use cognit::config::{
    McpOAuthClientAuthMethod, McpOAuthConfig, McpPermissionLevel, McpServerConfig,
    McpTransportConfig, McpTrustLevel,
};
pub use cognit::config::{
    McpOAuthClientAuthMethod, McpOAuthConfig, McpPermissionLevel, McpServerConfig,
    McpTransportConfig, McpTrustLevel,
};
