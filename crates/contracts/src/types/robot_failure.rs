//! Typed failure facts for the governed robot state machine.
//!
//! Display text is audit detail only. Retry/replan/safe-stop policy branches on
//! [`RobotFailureClass`], never on provider or model strings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RobotFailureClass {
    ObservationUnavailable,
    PerceptionUnavailable,
    PolicyUnavailable,
    ProposalRejected,
    ExecutionRejected,
    ExecutionFailed,
    ExecutionTimedOut,
    Cancelled,
    ProviderDisconnected,
    VerificationTimeout,
    VerificationMismatch,
    Unsafe,
    RetryBudgetExhausted,
    ReplanBudgetExhausted,
    RepeatedFailure,
    PersistenceFailure,
    SettlementFailure,
    SafeStopFailure,
}

impl RobotFailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ObservationUnavailable => "observation_unavailable",
            Self::PerceptionUnavailable => "perception_unavailable",
            Self::PolicyUnavailable => "policy_unavailable",
            Self::ProposalRejected => "proposal_rejected",
            Self::ExecutionRejected => "execution_rejected",
            Self::ExecutionFailed => "execution_failed",
            Self::ExecutionTimedOut => "execution_timed_out",
            Self::Cancelled => "cancelled",
            Self::ProviderDisconnected => "provider_disconnected",
            Self::VerificationTimeout => "verification_timeout",
            Self::VerificationMismatch => "verification_mismatch",
            Self::Unsafe => "unsafe",
            Self::RetryBudgetExhausted => "retry_budget_exhausted",
            Self::ReplanBudgetExhausted => "replan_budget_exhausted",
            Self::RepeatedFailure => "repeated_failure",
            Self::PersistenceFailure => "persistence_failure",
            Self::SettlementFailure => "settlement_failure",
            Self::SafeStopFailure => "safe_stop_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotFailure {
    pub class: RobotFailureClass,
    /// Human-readable audit detail. Runtime policy must not classify this text.
    pub detail: String,
}

impl RobotFailure {
    pub fn new(class: RobotFailureClass, detail: impl Into<String>) -> Self {
        Self {
            class,
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_failure_roundtrips_with_stable_snake_case_class() {
        let failure = RobotFailure::new(
            RobotFailureClass::ProviderDisconnected,
            "bridge channel closed",
        );
        let value = serde_json::to_value(&failure).unwrap();
        assert_eq!(value["class"], "provider_disconnected");
        assert_eq!(
            serde_json::from_value::<RobotFailure>(value).unwrap(),
            failure
        );
    }
}
