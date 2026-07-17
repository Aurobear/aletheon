use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::tools::PermissionLevel;

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
    /// Per-server permission overrides. Key is server_name, value is
    /// a PermissionLevel override (L0=ReadOnly, L1=Sandboxed, L2=SystemModify, L3=Destructive).
    /// Overrides the default trust→permission mapping for specific tools.
    #[serde(default)]
    pub permission_overrides: HashMap<String, PermissionLevel>,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,
    pub trust: McpTrustLevel,
    pub enabled: bool,
    /// Name of the environment variable that holds the bearer token.
    ///
    /// The variable is resolved during connection, not deserialization.
    /// An absent variable when a name is configured is a connection error.
    #[serde(default)]
    pub bearer_token_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum McpTransportConfig {
    Stdio { command: String, args: Vec<String> },
    StreamableHttp { url: String },
    Sse { url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpTrustLevel {
    LocalTrusted,
    RemoteTrusted,
    Untrusted,
}
