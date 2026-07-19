//! MCP configuration types — canonical source for both corpus and cognit.
//! Extracted to break the corpus → cognit circular dependency (Wave 2B).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,
    pub trust: McpTrustLevel,
    pub enabled: bool,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
    #[serde(default)]
    pub request_timeout_ms: Option<u64>,
    #[serde(default = "default_health_check")]
    pub health_check_interval_sec: u64,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub denylist: Vec<String>,
    #[serde(default)]
    pub permission_overrides: HashMap<String, McpPermissionLevel>,
}

fn default_health_check() -> u64 { 30 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpPermissionLevel { L0, L1, L2, L3 }

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthConfig {
    #[serde(default)] pub enabled: bool,
    pub client_id_env: String,
    #[serde(default)] pub client_secret_env: Option<String>,
    pub redirect_uri: String,
    #[serde(default)] pub scopes: Vec<String>,
    #[serde(default)] pub token_endpoint_auth_method: McpOAuthClientAuthMethod,
    #[serde(default)] pub issuer: Option<String>,
    #[serde(default)] pub authorization_endpoint: Option<String>,
    #[serde(default)] pub token_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthClientAuthMethod { #[default] None, ClientSecretBasic, ClientSecretPost }

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTransportConfig {
    Stdio { command: String, args: Vec<String> },
    StreamableHttp { url: String },
    Sse { url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum McpTrustLevel { LocalTrusted, RemoteTrusted, Untrusted }
