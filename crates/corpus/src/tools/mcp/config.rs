use std::collections::HashMap;

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

use crate::tools::PermissionLevel;

/// Canonical MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(from = "McpServerConfigWire")]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,
    pub trust: McpTrustLevel,
    pub enabled: bool,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    /// Explicit opt-in OAuth configuration. A configured bearer token takes
    /// precedence when both are present.
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
    #[serde(default)]
    pub request_timeout_ms: Option<u64>,
    #[serde(default = "default_mcp_health_check_interval_sec")]
    pub health_check_interval_sec: u64,
    /// Tool names exposed by this server. Empty means all discovered tools.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Tool names never exposed by this server. Deny entries take precedence.
    #[serde(default)]
    pub denylist: Vec<String>,
    /// Per-tool permission levels, keyed by the server-advertised tool name or
    /// its final registered name.
    #[serde(default)]
    pub permission_overrides: std::collections::HashMap<String, McpPermissionLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpPermissionLevel {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Environment variable containing the OAuth client id.
    pub client_id_env: String,
    /// Environment variable containing the client secret, when confidential
    /// client authentication is selected. Raw secrets are never configured.
    #[serde(default)]
    pub client_secret_env: Option<String>,
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: McpOAuthClientAuthMethod,
    /// RFC 8414 issuer/base URL. Discovery is authoritative when supplied.
    #[serde(default)]
    pub issuer: Option<String>,
    /// Explicit fallback endpoints for providers without discovery.
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthClientAuthMethod {
    #[default]
    None,
    ClientSecretBasic,
    ClientSecretPost,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTransportConfig {
    Stdio { command: String, args: Vec<String> },
    StreamableHttp { url: String },
    Sse { url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTrustLevel {
    LocalTrusted,
    RemoteTrusted,
    Untrusted,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum McpTransportWire {
    Canonical(McpTransportConfig),
    Legacy(String),
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct McpServerConfigWire {
    name: String,
    #[serde(default = "default_mcp_transport_wire")]
    transport: McpTransportWire,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default = "default_mcp_trust")]
    trust: McpTrustLevel,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    bearer_token_env: Option<String>,
    #[serde(default)]
    oauth: Option<McpOAuthConfig>,
    #[serde(default)]
    request_timeout_ms: Option<u64>,
    #[serde(default = "default_mcp_health_check_interval_sec")]
    health_check_interval_sec: u64,
    #[serde(default, alias = "allowlist")]
    tool_allowlist: Vec<String>,
    #[serde(default, alias = "denylist")]
    tool_denylist: Vec<String>,
    #[serde(default)]
    permission_overrides: std::collections::HashMap<String, McpPermissionLevel>,
}

fn default_mcp_transport_wire() -> McpTransportWire {
    McpTransportWire::Legacy("stdio".to_string())
}
fn default_mcp_trust() -> McpTrustLevel {
    McpTrustLevel::LocalTrusted
}
impl From<McpServerConfigWire> for McpServerConfig {
    fn from(wire: McpServerConfigWire) -> Self {
        let transport = match wire.transport {
            McpTransportWire::Canonical(value) => value,
            McpTransportWire::Legacy(value) if value == "http" => {
                McpTransportConfig::StreamableHttp {
                    url: wire.url.unwrap_or_default(),
                }
            }
            McpTransportWire::Legacy(value) if value == "sse" => McpTransportConfig::Sse {
                url: wire.url.unwrap_or_default(),
            },
            McpTransportWire::Legacy(_) => McpTransportConfig::Stdio {
                command: wire.command.unwrap_or_default(),
                args: wire.args,
            },
        };
        Self {
            name: wire.name,
            transport,
            trust: wire.trust,
            enabled: wire.enabled,
            bearer_token_env: wire.bearer_token_env,
            oauth: wire.oauth,
            request_timeout_ms: wire.request_timeout_ms,
            health_check_interval_sec: wire.health_check_interval_sec,
            allowlist: wire.tool_allowlist,
            denylist: wire.tool_denylist,
            permission_overrides: wire.permission_overrides,
        }
    }
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransportConfig::Stdio {
                command: String::new(),
                args: Vec::new(),
            },
            trust: McpTrustLevel::LocalTrusted,
            enabled: true,
            bearer_token_env: None,
            oauth: None,
            request_timeout_ms: None,
            health_check_interval_sec: default_mcp_health_check_interval_sec(),
            allowlist: Vec::new(),
            denylist: Vec::new(),
            permission_overrides: std::collections::HashMap::new(),
        }
    }
}

fn default_mcp_health_check_interval_sec() -> u64 {
    30
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_transport_and_oauth_schema_remain_compatible() {
        let configured: McpServerConfig = toml::from_str(
            r#"
name = "search"
transport = "http"
url = "https://mcp.example.test/rpc"

[oauth]
client_id_env = "MCP_CLIENT_ID"
redirect_uri = "http://127.0.0.1:8765/callback"
issuer = "https://issuer.example.test"
"#,
        )
        .unwrap();
        assert!(matches!(
            configured.transport,
            McpTransportConfig::StreamableHttp { .. }
        ));
        let oauth = configured.oauth.unwrap();
        assert!(!oauth.enabled);
        assert_eq!(
            oauth.token_endpoint_auth_method,
            McpOAuthClientAuthMethod::None
        );
    }

    #[test]
    fn inline_oauth_secrets_and_unknown_fields_are_rejected() {
        let unknown = r#"
name = "search"
transport = "http"
url = "https://mcp.example.test/rpc"
[oauth]
enabled = true
client_id_env = "MCP_CLIENT_ID"
redirect_uri = "http://127.0.0.1:8765/callback"
client_secret = "must-not-be-inline"
"#;
        assert!(toml::from_str::<McpServerConfig>(unknown).is_err());
    }

    #[test]
    fn server_tool_policy_deserializes_at_the_corpus_boundary() {
        let configured: McpServerConfig = toml::from_str(
            r#"
name = "external"
transport = "http"
url = "https://mcp.example.test/rpc"
allowlist = ["search"]
denylist = ["search.delete"]
[permission_overrides]
search = "l0"
"#,
        )
        .unwrap();
        assert_eq!(configured.allowlist, ["search"]);
        assert_eq!(configured.denylist, ["search.delete"]);
        assert_eq!(
            configured.permission_overrides.get("search"),
            Some(&McpPermissionLevel::L0)
        );
    }
}
