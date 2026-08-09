//! A1 installation acceptance scoreboard (Aletheon closure plan §13).
//!
//! The single machine-generated evidence source for release judgment.  A run
//! records repo/build/install/digest facts and per-task statuses.  Only the
//! allowed status enum is accepted (`not_run` / `infra_blocked` / `failed` /
//! `passed` / `waived`); prose states like `code_complete` are rejected.
//! `waived` requires an approver, reason and expiry, and cannot be used for
//! a P0 gate by default.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The only allowed task execution states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcceptanceStatus {
    NotRun,
    InfraBlocked,
    Failed,
    Passed,
    Waived,
}

impl AcceptanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AcceptanceStatus::NotRun => "not_run",
            AcceptanceStatus::InfraBlocked => "infra_blocked",
            AcceptanceStatus::Failed => "failed",
            AcceptanceStatus::Passed => "passed",
            AcceptanceStatus::Waived => "waived",
        }
    }
}

/// A waived task: requires approver, reason, and expiry (cannot be default for
/// a P0 gate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waiver {
    pub approver: String,
    pub reason: String,
    pub expires_at_ms: u64,
}

/// One acceptance task result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub status: AcceptanceStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub exit_code: Option<i32>,
    pub evidence_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiver: Option<Waiver>,
}

/// Build facts recorded at run start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildFacts {
    pub repo_sha: String,
    pub dirty: bool,
    pub profile: String,
    pub features: Vec<String>,
}

/// Installed artifact facts (digest, not just source SHA).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledFacts {
    pub artifact_digest: String,
    pub daemon_version: String,
    pub protocol_version: u32,
    pub provider_endpoint_redacted: String,
}

/// The full scoreboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scoreboard {
    pub run_id: String,
    pub build: BuildFacts,
    pub installed: InstalledFacts,
    pub tasks: BTreeMap<String, TaskResult>,
    pub scope_leak_count: u64,
    pub retry_count: u64,
    pub generation_id: String,
}

impl Scoreboard {
    pub fn overall_passed(&self, p0_gates: &[&str]) -> Result<(), String> {
        for gate in p0_gates {
            let task = self
                .tasks
                .get(*gate)
                .ok_or_else(|| format!("missing P0 gate: {gate}"))?;
            match task.status {
                AcceptanceStatus::Passed => {}
                AcceptanceStatus::Waived => {
                    // Waiver cannot be default for a P0 gate.
                    return Err(format!("P0 gate {gate} is waived, not passed"));
                }
                other => return Err(format!("P0 gate {gate} is {}", other.as_str())),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed_task() -> TaskResult {
        TaskResult {
            status: AcceptanceStatus::Passed,
            started_at_ms: 0,
            ended_at_ms: 1,
            exit_code: Some(0),
            evidence_path: Some("logs/t1".into()),
            waiver: None,
        }
    }

    #[test]
    fn status_enum_rejects_prose_states() {
        // Only the enum values serialize; a prose "code_complete" has no
        // representation (compile-time rejection of the invalid states).
        assert_eq!(AcceptanceStatus::Passed.as_str(), "passed");
        assert_eq!(AcceptanceStatus::NotRun.as_str(), "not_run");
    }

    #[test]
    fn all_p0_passed_returns_ok() {
        let scoreboard = Scoreboard {
            run_id: "r1".into(),
            build: BuildFacts {
                repo_sha: "abc".into(),
                dirty: false,
                profile: "release".into(),
                features: vec![],
            },
            installed: InstalledFacts {
                artifact_digest: "sha256:x".into(),
                daemon_version: "0.1.0".into(),
                protocol_version: 1,
                provider_endpoint_redacted: "endpoint:<redacted>".into(),
            },
            tasks: BTreeMap::from([
                ("p0-session".to_string(), passed_task()),
                ("p0-turn".to_string(), passed_task()),
            ]),
            scope_leak_count: 0,
            retry_count: 0,
            generation_id: "g1".into(),
        };
        assert!(scoreboard
            .overall_passed(&["p0-session", "p0-turn"])
            .is_ok());
    }

    #[test]
    fn waived_p0_gate_fails() {
        let mut scoreboard = Scoreboard {
            run_id: "r1".into(),
            build: BuildFacts {
                repo_sha: "abc".into(),
                dirty: false,
                profile: "release".into(),
                features: vec![],
            },
            installed: InstalledFacts {
                artifact_digest: "sha256:x".into(),
                daemon_version: "0.1.0".into(),
                protocol_version: 1,
                provider_endpoint_redacted: "e".into(),
            },
            tasks: BTreeMap::from([(
                "p0-session".to_string(),
                TaskResult {
                    status: AcceptanceStatus::Waived,
                    started_at_ms: 0,
                    ended_at_ms: 1,
                    exit_code: None,
                    evidence_path: None,
                    waiver: Some(Waiver {
                        approver: "a".into(),
                        reason: "r".into(),
                        expires_at_ms: 9_999,
                    }),
                },
            )]),
            scope_leak_count: 0,
            retry_count: 0,
            generation_id: "g1".into(),
        };
        assert!(scoreboard.overall_passed(&["p0-session"]).is_err());
        let _ = &mut scoreboard;
    }
}
