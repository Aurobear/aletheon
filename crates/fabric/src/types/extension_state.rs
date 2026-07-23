//! Activation state machine and health types for the Aletheon extension platform.
//!
//! These types are used by the activation transaction pipeline (Phase 3+),
//! health monitoring (Phase 5+), and Metacog observation (Phase 6+).

use serde::{Deserialize, Serialize};

use super::time::WallTime;

// ---------------------------------------------------------------------------
// Activation state machine
// ---------------------------------------------------------------------------

/// The lifecycle of a single extension activation from discovery through
/// active use, failure, rollback, or disablement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationState {
    /// Package has been discovered but not yet validated.
    Discovered,
    /// Manifest and archive integrity validated.
    Validated,
    /// Content extracted into isolated staging.
    Staged,
    /// Waiting for operator or policy approval.
    PendingApproval,
    /// Running compatibility and health probe.
    Probing,
    /// Extension is active and serving capabilities.
    Active,
    /// Activation was explicitly rejected.
    Rejected {
        reason: String,
    },
    /// Extension failed validation or health probe; isolated, not serving.
    Quarantined {
        reason: String,
    },
    /// Extension is active but one or more capabilities are degraded.
    Degraded {
        reason: String,
    },
    /// In the process of rolling back to a previous known-good version.
    RollingBack,
    /// Successfully rolled back to previous version.
    RolledBack,
    /// Explicitly disabled by operator or policy.
    Disabled,
}

impl ActivationState {
    /// Returns true if this state indicates the extension is currently serving.
    pub fn is_serving(&self) -> bool {
        matches!(self, Self::Active | Self::Degraded { .. })
    }

    /// Returns true if this state is terminal for the current activation attempt.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Active
                | Self::Rejected { .. }
                | Self::Quarantined { .. }
                | Self::RolledBack
                | Self::Disabled
        )
    }
}

// ---------------------------------------------------------------------------
// Activation transition
// ---------------------------------------------------------------------------

/// Immutable record of a single state transition in the activation pipeline.
/// Every transition is persisted as a transaction receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationTransition {
    /// Unique transaction identifier linking related transitions.
    pub transaction_id: String,
    pub old_state: ActivationState,
    pub new_state: ActivationState,
    /// Machine-readable reason code (e.g. "validation_failed", "probe_timeout").
    pub reason_code: String,
    /// Human-readable detail for diagnostics.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason_detail: String,
    pub timestamp: WallTime,
    /// SHA-256 hash of the package content at time of transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// Health state
// ---------------------------------------------------------------------------

/// Aggregated health of an extension or its runtime instances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// All capabilities operating normally.
    Healthy,
    /// One or more capabilities degraded but extension still serving.
    Degraded {
        /// List of failure descriptions (capability name + error summary).
        failures: Vec<String>,
    },
    /// Extension is not serving and requires operator intervention.
    Unhealthy {
        reason: String,
    },
}

impl HealthState {
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_wall_time() -> WallTime {
        WallTime(1_700_000_000_000)
    }

    #[test]
    fn activation_state_serde_snake_case() {
        let cases = [
            (ActivationState::Discovered, "\"discovered\""),
            (ActivationState::Validated, "\"validated\""),
            (ActivationState::Active, "\"active\""),
            (ActivationState::Disabled, "\"disabled\""),
        ];
        for (state, expected) in cases {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn activation_state_with_data_fields() {
        let rejected = ActivationState::Rejected {
            reason: "permission denied".into(),
        };
        let json = serde_json::to_string(&rejected).unwrap();
        assert!(json.contains("rejected"));
        assert!(json.contains("permission denied"));
        let rt: ActivationState = serde_json::from_str(&json).unwrap();
        assert_eq!(rt, rejected);

        let quarantined = ActivationState::Quarantined {
            reason: "health probe timeout".into(),
        };
        let json = serde_json::to_string(&quarantined).unwrap();
        assert!(json.contains("quarantined"));
        assert!(json.contains("health probe timeout"));
    }

    #[test]
    fn activation_state_serving_and_terminal() {
        assert!(ActivationState::Active.is_serving());
        assert!(ActivationState::Degraded {
            reason: "slow".into()
        }
        .is_serving());
        assert!(!ActivationState::Discovered.is_serving());
        assert!(!ActivationState::Quarantined {
            reason: "fail".into()
        }
        .is_serving());

        assert!(ActivationState::Active.is_terminal());
        assert!(ActivationState::Rejected {
            reason: "no".into()
        }
        .is_terminal());
        assert!(!ActivationState::Probing.is_terminal());
        assert!(!ActivationState::RollingBack.is_terminal());
    }

    #[test]
    fn activation_transition_round_trip() {
        let tx = ActivationTransition {
            transaction_id: "tx-001".into(),
            old_state: ActivationState::Staged,
            new_state: ActivationState::Active,
            reason_code: "health_probe_passed".into(),
            reason_detail: String::new(),
            timestamp: test_wall_time(),
            package_hash: Some(
                "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".into(),
            ),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let rt: ActivationTransition = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.transaction_id, "tx-001");
        assert_eq!(rt.old_state, ActivationState::Staged);
        assert_eq!(rt.new_state, ActivationState::Active);
        assert_eq!(rt.reason_code, "health_probe_passed");
        assert!(rt.package_hash.is_some());
    }

    #[test]
    fn health_state_serde() {
        let healthy = HealthState::Healthy;
        let json = serde_json::to_string(&healthy).unwrap();
        assert_eq!(json, "\"healthy\"");

        let degraded = HealthState::Degraded {
            failures: vec!["connector timeout".into(), "slow response".into()],
        };
        let json = serde_json::to_string(&degraded).unwrap();
        assert!(json.contains("degraded"));
        assert!(json.contains("connector timeout"));
        let rt: HealthState = serde_json::from_str(&json).unwrap();
        assert!(!rt.is_healthy());

        let unhealthy = HealthState::Unhealthy {
            reason: "crash loop".into(),
        };
        assert!(!unhealthy.is_healthy());
    }
}
