//! Versioned contracts for governed extension lifecycle and MCP connectors.

use std::path::{Component, Path, PathBuf};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const EXTENSION_PROTOCOL_SCHEMA_V1: u16 = 1;
pub const MAX_CONNECTOR_REQUEST_TIMEOUT_MS: u64 = 120_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionEnableRequestV1 {
    pub schema_version: u16,
    pub package_id: String,
    pub approve_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionPackagePathRequestV1 {
    pub schema_version: u16,
    pub path: PathBuf,
    pub trust_workspace: bool,
    pub approve_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionPackageIdRequestV1 {
    pub schema_version: u16,
    pub package_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtensionMutationReceiptV1 {
    pub schema_version: u16,
    pub operation: String,
    pub actor: String,
    pub package_id: String,
    pub package_version: Option<String>,
    pub package_hash: Option<String>,
    pub previous_snapshot_digest: String,
    pub snapshot_digest: String,
    pub permission_approved: bool,
    pub health: String,
    pub evidence_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpConnectorTransportV1 {
    Stdio { command: String, args: Vec<String> },
    StreamableHttp { url: String },
    Sse { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpConnectorManifestV1 {
    pub schema_version: u16,
    pub id: String,
    pub transport: McpConnectorTransportV1,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub allowed_resources: Vec<String>,
}

impl McpConnectorManifestV1 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == EXTENSION_PROTOCOL_SCHEMA_V1,
            "unsupported MCP connector schema version {}",
            self.schema_version
        );
        anyhow::ensure!(!self.id.trim().is_empty(), "MCP connector id is empty");
        anyhow::ensure!(
            (1..=MAX_CONNECTOR_REQUEST_TIMEOUT_MS).contains(&self.request_timeout_ms),
            "MCP connector request timeout is outside 1..={MAX_CONNECTOR_REQUEST_TIMEOUT_MS}ms"
        );

        match &self.transport {
            McpConnectorTransportV1::Stdio { command, .. } => {
                validate_package_relative_command(command)?;
            }
            McpConnectorTransportV1::StreamableHttp { url }
            | McpConnectorTransportV1::Sse { url } => validate_network_url(url)?,
        }

        if let Some(name) = &self.bearer_token_env {
            let pattern = Regex::new(r"^[A-Z][A-Z0-9_]*$")
                .expect("static bearer-token environment pattern is valid");
            anyhow::ensure!(
                pattern.is_match(name),
                "bearer_token_env must name a host environment variable"
            );
        }
        anyhow::ensure!(
            self.allowed_tools
                .iter()
                .all(|name| !name.trim().is_empty())
                && self
                    .allowed_resources
                    .iter()
                    .all(|name| !name.trim().is_empty()),
            "connector allowlists cannot contain empty entries"
        );
        Ok(())
    }
}

pub fn default_request_timeout_ms() -> u64 {
    30_000
}

fn validate_package_relative_command(command: &str) -> anyhow::Result<()> {
    let path = Path::new(command);
    anyhow::ensure!(!command.trim().is_empty(), "stdio command is empty");
    anyhow::ensure!(
        !path.is_absolute(),
        "stdio command must be package-relative"
    );
    anyhow::ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "stdio command contains an escaping or non-normal path component"
    );
    anyhow::ensure!(
        path.starts_with("payload") && path.components().count() > 1,
        "stdio command must be located under payload/"
    );
    Ok(())
}

fn validate_network_url(url: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !url.chars().any(char::is_whitespace),
        "connector URL contains whitespace"
    );
    let authority_and_path = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("connector URL must use http or https"))?;
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    anyhow::ensure!(!authority.is_empty(), "connector URL host is empty");
    anyhow::ensure!(
        !authority.contains('@'),
        "connector URL must not contain inline credentials"
    );
    Ok(())
}
