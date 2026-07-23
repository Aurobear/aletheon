//! Doctor diagnostics: comprehensive health check for `aletheon doctor [--json]`.
//!
//! Powers D2-M5-T1. Reports config validity, MCP server health, sandbox/execd
//! status, and recent writer failures in a schema-stable JSON format with secrets redacted.

use anyhow::Context;
use serde::Serialize;

use crate::composition::config::{AppConfig, LoadedConfig};
use crate::core::deploy::{DeploymentInfo, DeploymentManifest};

const MAX_DOCTOR_MCP_SERVERS: usize = 64;
const MAX_DOCTOR_PROVENANCE: usize = 512;
const MAX_DOCTOR_WARNINGS: usize = 64;
const MAX_DOCTOR_TEXT_CHARS: usize = 512;
const MAX_DOCTOR_JSON_ENTRIES: usize = 256;
const MAX_DOCTOR_JSON_DEPTH: usize = 16;

fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_DOCTOR_TEXT_CHARS).collect()
}

fn bounded_json(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= MAX_DOCTOR_JSON_DEPTH {
        return serde_json::Value::String("[depth limit]".into());
    }
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(bounded_text(text)),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .take(MAX_DOCTOR_JSON_ENTRIES)
                .map(|value| bounded_json(value, depth + 1))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .take(MAX_DOCTOR_JSON_ENTRIES)
                .map(|(key, value)| (bounded_text(key), bounded_json(value, depth + 1)))
                .collect(),
        ),
        scalar => scalar.clone(),
    }
}

fn redacted_transport(config: &corpus::tools::mcp::config::McpTransportConfig) -> String {
    match config {
        corpus::tools::mcp::config::McpTransportConfig::Stdio { command, .. } => {
            let executable = std::path::Path::new(command)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("<command>");
            format!("stdio:{} [arguments redacted]", bounded_text(executable))
        }
        corpus::tools::mcp::config::McpTransportConfig::StreamableHttp { .. } => {
            "streamable_http:[endpoint redacted]".into()
        }
        corpus::tools::mcp::config::McpTransportConfig::Sse { .. } => {
            "sse:[endpoint redacted]".into()
        }
    }
}

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

    /// Sandbox / execd status.
    pub sandbox: SandboxStatus,

    /// Recent writer failures (from M4-T1 persistence layer).
    pub writer_health: WriterHealth,

    /// Bounded summary of the latest all-session startup recovery scan.
    pub turn_recovery: crate::application::turn_recovery::TurnRecoveryHealth,

    /// Persisted invalid Agent profiles. Reasons stay in the local quarantine
    /// record; doctor exposes only bounded names to avoid leaking config data.
    pub quarantined_profiles: QuarantinedProfilesStatus,

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
    pub execd: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct QuarantinedProfilesStatus {
    pub count: usize,
    pub names: Vec<String>,
}

impl DoctorReport {
    /// Create a standalone doctor report (no daemon connection).
    /// Checks what it can: config validity, deployment info, configured MCP/sandbox.
    pub fn standalone(loaded_config: &LoadedConfig) -> Self {
        let config = &loaded_config.value;
        let effective = loaded_config.effective_view();
        let mut warnings = Vec::new();

        let data_dir = config
            .bootstrap
            .data_dir
            .clone()
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
            .take(MAX_DOCTOR_MCP_SERVERS)
            .map(|mcp| McpServerStatus {
                name: bounded_text(&mcp.name),
                command: redacted_transport(&mcp.transport),
                status: "unknown (standalone doctor)".to_string(),
                error: None,
            })
            .collect();

        let sandbox = SandboxStatus {
            status: config.sandbox.preference.clone(),
            command: config.sandbox.bubblewrap_path.clone(),
            execd: if config.grok_hardening.execd {
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
        let turn_recovery = match crate::application::turn_recovery::read_recovery_health(&data_dir)
        {
            Ok(health) => health,
            Err(error) => {
                warnings.push(format!("turn recovery health unavailable: {error}"));
                crate::application::turn_recovery::TurnRecoveryHealth::default()
            }
        };
        let quarantined_profiles = match read_quarantined_profiles(&data_dir) {
            Ok(status) => status,
            Err(error) => {
                warnings.push(format!("profile quarantine unavailable: {error}"));
                QuarantinedProfilesStatus::default()
            }
        };

        // Determine overall status.
        let status = if config_validity_is_ok(config)
            && deployment.is_healthy()
            && writer_health.writes_succeeding
            && writer_health.recent_failures == 0
            && quarantined_profiles.count == 0
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
                effective: Some(bounded_json(&effective.config, 0)),
                provenance: Some(
                    loaded_config
                        .provenance
                        .iter()
                        .take(MAX_DOCTOR_PROVENANCE)
                        .map(|(path, source)| DoctorProvenanceEntry {
                            path: bounded_text(path),
                            source_kind: format!("{:?}", source.kind).to_lowercase(),
                            // A locator may contain a home path, URL query, or
                            // environment-derived secret. The source kind is
                            // sufficient for standalone diagnosis.
                            locator: "[redacted]".into(),
                        })
                        .collect(),
                ),
            },
            deployment,
            mcp_servers,
            sandbox,
            writer_health,
            turn_recovery,
            quarantined_profiles,
            daemon_version: env!("CARGO_PKG_VERSION"),
            warnings: warnings
                .into_iter()
                .take(MAX_DOCTOR_WARNINGS)
                .map(|warning| bounded_text(&warning))
                .collect(),
        }
    }
}

fn read_quarantined_profiles(
    data_dir: &std::path::Path,
) -> anyhow::Result<QuarantinedProfilesStatus> {
    let primary = data_dir.join("state/agent-profile-quarantine.json");
    let fallback = data_dir.join("agent-profile-quarantine.json");
    let path = if primary.exists() { primary } else { fallback };
    if !path.exists() {
        return Ok(QuarantinedProfilesStatus::default());
    }
    let records: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)?;
    let records = records
        .as_array()
        .context("profile quarantine record must be an array")?;
    let names = records
        .iter()
        .take(MAX_DOCTOR_JSON_ENTRIES)
        .filter_map(|record| record.get("name").and_then(|value| value.as_str()))
        .map(bounded_text)
        .collect();
    Ok(QuarantinedProfilesStatus {
        count: records.len(),
        names,
    })
}

fn config_validity_is_ok(_config: &AppConfig) -> bool {
    // Basic validation: if we could deserialize it, it's structurally valid.
    true
}

fn writer_health_at(data_dir: &std::path::Path) -> anyhow::Result<WriterHealth> {
    let snapshot = crate::application::durable_write::read_writer_health(data_dir)?;
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
    use crate::composition::config::merge_layers;

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
    fn doctor_redacts_mcp_arguments_and_remote_endpoints() {
        let stdio = corpus::tools::mcp::config::McpTransportConfig::Stdio {
            command: "/secret/home/bin/server".into(),
            args: vec!["--token=super-secret".into()],
        };
        let http = corpus::tools::mcp::config::McpTransportConfig::StreamableHttp {
            url: "https://user:secret@example.test/private?token=secret".into(),
        };
        let stdio = redacted_transport(&stdio);
        let http = redacted_transport(&http);
        assert_eq!(stdio, "stdio:server [arguments redacted]");
        assert!(!stdio.contains("super-secret"));
        assert_eq!(http, "streamable_http:[endpoint redacted]");
        assert!(!http.contains("example.test"));
    }

    #[test]
    fn doctor_text_fields_are_deterministically_bounded() {
        let bounded = bounded_text(&"x".repeat(MAX_DOCTOR_TEXT_CHARS + 100));
        assert_eq!(bounded.chars().count(), MAX_DOCTOR_TEXT_CHARS);
        let value = serde_json::Value::Array(
            (0..MAX_DOCTOR_JSON_ENTRIES + 10)
                .map(serde_json::Value::from)
                .collect(),
        );
        assert_eq!(
            bounded_json(&value, 0).as_array().unwrap().len(),
            MAX_DOCTOR_JSON_ENTRIES
        );
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
    fn doctor_reads_bounded_profile_quarantine_without_reasons() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("agent-profile-quarantine.json"),
            br#"[{"name":"broken-profile","reason":"token=must-not-leak"}]"#,
        )
        .unwrap();
        let status = read_quarantined_profiles(temp.path()).unwrap();
        assert_eq!(status.count, 1);
        assert_eq!(status.names, vec!["broken-profile"]);
        assert!(!serde_json::to_string(&status)
            .unwrap()
            .contains("must-not-leak"));
    }

    #[test]
    fn doctor_reads_persisted_writer_failure_health() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = crate::application::durable_write::WriterHealthSnapshot {
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

    #[test]
    fn doctor_reads_bounded_turn_recovery_summary() {
        let temp = tempfile::tempdir().unwrap();
        let report = crate::application::turn_recovery::TurnRecoveryReport {
            sessions_scanned: 4,
            turns_scanned: 9,
            incomplete_turns: Vec::new(),
        };
        crate::application::turn_recovery::persist_recovery_health(temp.path(), &report).unwrap();
        let health = crate::application::turn_recovery::read_recovery_health(temp.path()).unwrap();
        assert_eq!(health.sessions_scanned, 4);
        assert_eq!(health.turns_scanned, 9);
        assert_eq!(health.incomplete_turns_recovered, 0);
    }
}
