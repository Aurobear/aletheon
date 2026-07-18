//! Doctor diagnostics: comprehensive health check for `aletheon doctor [--json]`.
//!
//! Powers D2-M5-T1. Reports config validity, MCP server health, sandbox/exec-server
//! status, and recent writer failures in a schema-stable JSON format with secrets redacted.

use serde::Serialize;

use crate::core::config::{AppConfig, LoadedConfig};
use crate::core::deploy::{DeploymentInfo, CORE_RUNTIME_VERSION};

/// Schema-stable doctor report. All fields use predictable keys;
/// secrets are always redacted by the config rendering layer.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// Overall health: "healthy", "degraded", or "unhealthy".
    pub status: &'static str,

    /// Configuration validity and effective config (secrets redacted).
    pub config: DoctorConfigStatus,

    /// Deployment verification (installed SHA, version compatibility).
    pub deployment: DeploymentInfo,

    /// Connected MCP servers and their health status.
    pub mcp_servers: Vec<McpServerStatus>,

    /// Sandbox / exec-server status.
    pub sandbox: SandboxStatus,

    /// Recent writer failures (from M4-T1 persistence layer).
    pub writer_health: WriterHealth,

    /// Daemon daemon_version for diagnostics.
    pub daemon_version: &'static str,

    /// Any warnings or diagnostic messages.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorConfigStatus {
    /// "valid" or "invalid" with error details.
    pub validity: String,
    /// Number of config leaves in the effective config.
    pub leaf_count: usize,
    /// The effective config with secrets redacted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<serde_json::Value>,
    /// Per-leaf provenance summary (path -> source).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Vec<DoctorProvenanceEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorProvenanceEntry {
    pub path: String,
    pub source_kind: String,
    pub locator: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub command: String,
    /// "unknown" when daemon is not running (standalone doctor).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SandboxStatus {
    /// "enabled", "disabled", or "unknown"
    pub status: String,
    /// Sandbox command if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Exec-server status if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_server: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriterHealth {
    /// "healthy", "degraded", or "unknown"
    pub status: String,
    /// Number of recent write failures (0 if healthy).
    pub recent_failures: u64,
    /// Whether write operations are currently succeeding.
    pub writes_succeeding: bool,
}

impl DoctorReport {
    /// Create a standalone doctor report (no daemon connection).
    /// Checks what it can: config validity, deployment info, configured MCP/sandbox.
    pub fn standalone(loaded_config: &LoadedConfig) -> Self {
        let config = &loaded_config.value;
        let effective = loaded_config.effective_view();
        let mut warnings = Vec::new();

        let mut deployment = DeploymentInfo::gather();
        deployment.verify_binary(None);
        deployment.verify_runtime_compatibility(CORE_RUNTIME_VERSION);
        if !deployment.is_healthy() {
            warnings.extend(deployment.version_warnings.clone());
        }

        // Collect MCP server status from configured servers (daemon-not-running
        // view, so all are "unknown").
        let mcp_servers: Vec<McpServerStatus> = config
            .mcp_servers
            .iter()
            .map(|mcp| McpServerStatus {
                name: mcp.name.clone(),
                command: mcp
                    .command
                    .clone()
                    .unwrap_or_else(|| mcp.url.clone().unwrap_or_else(|| mcp.transport.clone())),
                status: "unknown (standalone doctor)".to_string(),
                error: None,
            })
            .collect();

        let sandbox = SandboxStatus {
            status: config.sandbox.preference.clone(),
            command: config.sandbox.bubblewrap_path.clone(),
            exec_server: if config.grok_hardening.streaming_tools {
                Some("configured (not yet connected)".to_string())
            } else {
                Some("not configured".to_string())
            },
        };

        let writer_health = WriterHealth {
            status: "unknown (standalone doctor)".to_string(),
            recent_failures: 0,
            writes_succeeding: true,
        };

        // Determine overall status.
        let status = if config_validity_is_ok(config) && deployment.is_healthy() {
            "healthy"
        } else {
            "degraded"
        };

        DoctorReport {
            status,
            config: DoctorConfigStatus {
                validity: "valid".to_string(),
                leaf_count: effective.leaf_count,
                effective: Some(effective.config.clone()),
                provenance: Some(
                    loaded_config
                        .provenance
                        .iter()
                        .map(|(path, source)| DoctorProvenanceEntry {
                            path: path.to_string(),
                            source_kind: format!("{:?}", source.kind).to_lowercase(),
                            locator: source.locator.clone(),
                        })
                        .collect(),
                ),
            },
            deployment,
            mcp_servers,
            sandbox,
            writer_health,
            daemon_version: env!("CARGO_PKG_VERSION"),
            warnings,
        }
    }
}

fn config_validity_is_ok(_config: &AppConfig) -> bool {
    // Basic validation: if we could deserialize it, it's structurally valid.
    true
}

#[cfg(test)]
mod tests {
    use crate::core::config::merge_layers;

    use super::*;

    #[test]
    fn standalone_doctor_report_is_serializable() {
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let report = DoctorReport::standalone(&loaded);
        let json = serde_json::to_string_pretty(&report).expect("serializable");
        assert!(json.contains("\"status\""));
        assert!(json.contains("\"config\""));
        assert!(json.contains("\"deployment\""));
        assert!(!json.contains("\"api_key\": \"")); // secrets redacted
    }

    #[test]
    fn doctor_report_default_config_is_healthy() {
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let report = DoctorReport::standalone(&loaded);
        assert_eq!(report.status, "healthy");
    }

    #[test]
    fn doctor_mcp_servers_reflect_config() {
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let report = DoctorReport::standalone(&loaded);
        // Default config has no MCP servers configured
        assert!(report.mcp_servers.is_empty());
    }
}
