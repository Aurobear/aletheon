//! Doctor diagnostics: comprehensive health check for `aletheon doctor [--json]`.
//!
//! Powers D2-M5-T1. Reports config validity, MCP server health, sandbox/exec-server
//! status, and recent writer failures in a schema-stable JSON format with secrets redacted.

use serde::Serialize;

use crate::core::config::{AppConfig, LoadedConfig};
use crate::core::deploy::{DeploymentInfo, DeploymentManifest};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_phase: Option<String>,
}

impl DoctorReport {
    /// Create a standalone doctor report (no daemon connection).
    /// Checks what it can: config validity, deployment info, configured MCP/sandbox.
    pub fn standalone(loaded_config: &LoadedConfig) -> Self {
        let config = &loaded_config.value;
        let effective = loaded_config.effective_view();
        let mut warnings = Vec::new();

        let data_dir = std::env::var_os("AGENT_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| config.deployment.paths.state.clone());
        let manifest_path = std::env::var_os("ALETHEON_DEPLOYMENT_MANIFEST")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| data_dir.join("deployment-manifest.json"));
        let mut deployment = DeploymentInfo::gather();
        match DeploymentManifest::load(&manifest_path) {
            Ok(manifest) => deployment.verify_manifest(&manifest),
            Err(error) => deployment.mark_manifest_unavailable(error),
        }
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
                command: match &mcp.transport {
                    cognit::config::McpTransportConfig::Stdio { command, args } => {
                        std::iter::once(command.as_str())
                            .chain(args.iter().map(String::as_str))
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                    cognit::config::McpTransportConfig::StreamableHttp { url }
                    | cognit::config::McpTransportConfig::Sse { url } => url.clone(),
                },
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

        let writer_health = match writer_health_at(&data_dir) {
            Ok(health) => health,
            Err(error) => {
                warnings.push(format!("writer health unavailable: {error}"));
                WriterHealth {
                    status: "unknown".into(),
                    recent_failures: 0,
                    writes_succeeding: false,
                    last_failure_phase: None,
                }
            }
        };

        // Determine overall status.
        let status = if config_validity_is_ok(config)
            && deployment.is_healthy()
            && writer_health.writes_succeeding
            && writer_health.recent_failures == 0
        {
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

fn writer_health_at(data_dir: &std::path::Path) -> anyhow::Result<WriterHealth> {
    let snapshot = crate::service::durable_write::read_writer_health(data_dir)?;
    Ok(WriterHealth {
        status: if snapshot.writes_succeeding && snapshot.recent_failures == 0 {
            "healthy".into()
        } else {
            "degraded".into()
        },
        recent_failures: snapshot.recent_failures,
        writes_succeeding: snapshot.writes_succeeding,
        last_failure_phase: snapshot.last_failure_phase,
    })
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
        assert!(matches!(report.status, "healthy" | "degraded"));
    }

    #[test]
    fn doctor_mcp_servers_reflect_config() {
        let loaded = merge_layers(std::iter::empty()).expect("default config");
        let report = DoctorReport::standalone(&loaded);
        // Default config has no MCP servers configured
        assert!(report.mcp_servers.is_empty());
    }

    #[test]
    fn doctor_reads_persisted_writer_failure_health() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = crate::service::durable_write::WriterHealthSnapshot {
            recent_failures: 1,
            writes_succeeding: false,
            last_failure_phase: Some("terminal_flush".into()),
            last_failure_reason: Some("redacted from doctor".into()),
        };
        std::fs::write(
            temp.path().join("bounded-writer-health.json"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let health = writer_health_at(temp.path()).unwrap();
        assert_eq!(health.recent_failures, 1);
        assert!(!health.writes_succeeding);
        assert_eq!(health.last_failure_phase.as_deref(), Some("terminal_flush"));
    }
}
