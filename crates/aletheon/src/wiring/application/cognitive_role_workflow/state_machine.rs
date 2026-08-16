use ::contracts::cognitive_workflow::CognitiveRole;
use ::contracts::types::admission::RiskLevel;
use ::contracts::AgentRuntimeCapability;

pub(super) const MAX_FIX_ATTEMPTS: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UncertaintyLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvidenceSufficiency {
    Sufficient,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransitionReason {
    LowRiskEvidenceSufficient,
    MissingInvestigationEvidence,
    HighRiskPlanningRequired,
    StructuredArtifactAccepted,
    StructuredArtifactRejected,
    ValidationRequired,
    ValidationPassed,
    ValidationFailed,
    ReviewRequired,
    ReviewPassed,
    ReviewFailed,
    StructuredFailureRepair,
    RepairValidated,
    RepairBudgetExhausted,
    Cancelled,
}

impl TransitionReason {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::LowRiskEvidenceSufficient => "low_risk_evidence_sufficient",
            Self::MissingInvestigationEvidence => "missing_investigation_evidence",
            Self::HighRiskPlanningRequired => "high_risk_planning_required",
            Self::StructuredArtifactAccepted => "structured_artifact_accepted",
            Self::StructuredArtifactRejected => "structured_artifact_rejected",
            Self::ValidationRequired => "validation_required",
            Self::ValidationPassed => "validation_passed",
            Self::ValidationFailed => "validation_failed",
            Self::ReviewRequired => "review_required",
            Self::ReviewPassed => "review_passed",
            Self::ReviewFailed => "review_failed",
            Self::StructuredFailureRepair => "structured_failure_repair",
            Self::RepairValidated => "repair_validated",
            Self::RepairBudgetExhausted => "repair_budget_exhausted",
            Self::Cancelled => "cancelled",
        }
    }

    pub(super) fn detail(self, detail: &str) -> String {
        format!("{}: {detail}", self.code())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodingStagePlan {
    pub(super) roles: Vec<CognitiveRole>,
    pub(super) uncertainty: UncertaintyLevel,
    pub(super) reason: TransitionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AcceptanceStagePlan {
    pub(super) tester_required: bool,
    pub(super) reviewer_required: bool,
    pub(super) max_fix_attempts: u8,
}

pub(crate) fn classify_task_risk(capabilities: &[AgentRuntimeCapability]) -> RiskLevel {
    if capabilities
        .iter()
        .any(|capability| matches!(capability, AgentRuntimeCapability::DeviceCommand))
    {
        RiskLevel::SystemModify
    } else if capabilities.iter().any(|capability| {
        matches!(
            capability,
            AgentRuntimeCapability::CodeEdit
                | AgentRuntimeCapability::Shell
                | AgentRuntimeCapability::Test
                | AgentRuntimeCapability::Git
        )
    }) {
        RiskLevel::Sandboxed
    } else {
        RiskLevel::ReadOnly
    }
}

pub(super) fn coding_plan(
    risk: RiskLevel,
    workspace_scope: &[String],
    expected_evidence: &[String],
) -> CodingStagePlan {
    let evidence = if expected_evidence.is_empty() {
        EvidenceSufficiency::Sufficient
    } else {
        EvidenceSufficiency::Missing
    };
    match (risk, evidence, workspace_scope.is_empty()) {
        (RiskLevel::ReadOnly, EvidenceSufficiency::Sufficient, _) => CodingStagePlan {
            roles: vec![CognitiveRole::Executor],
            uncertainty: UncertaintyLevel::Low,
            reason: TransitionReason::LowRiskEvidenceSufficient,
        },
        (RiskLevel::ReadOnly, EvidenceSufficiency::Missing, _) => CodingStagePlan {
            roles: vec![CognitiveRole::Explorer, CognitiveRole::Executor],
            uncertainty: UncertaintyLevel::Medium,
            reason: TransitionReason::MissingInvestigationEvidence,
        },
        (_, _, true) | (RiskLevel::SystemModify | RiskLevel::Destructive, _, _) => {
            CodingStagePlan {
                roles: vec![
                    CognitiveRole::Planner,
                    CognitiveRole::Explorer,
                    CognitiveRole::Executor,
                ],
                uncertainty: UncertaintyLevel::High,
                reason: TransitionReason::HighRiskPlanningRequired,
            }
        }
        (RiskLevel::Sandboxed, _, false) => CodingStagePlan {
            roles: vec![CognitiveRole::Explorer, CognitiveRole::Executor],
            uncertainty: UncertaintyLevel::Medium,
            reason: TransitionReason::MissingInvestigationEvidence,
        },
    }
}

pub(super) fn acceptance_plan(
    risk: RiskLevel,
    expected_evidence: &[String],
) -> AcceptanceStagePlan {
    // M1: Tester/Reviewer are evidence-driven, not fixed-count.
    // Tester is required when risk is Sandboxed or higher, *or* when
    // specific validation evidence is expected but not yet produced.
    // Reviewer is required when risk is SystemModify or higher.
    AcceptanceStagePlan {
        tester_required: risk >= RiskLevel::Sandboxed || !expected_evidence.is_empty(),
        reviewer_required: risk >= RiskLevel::SystemModify,
        max_fix_attempts: MAX_FIX_ATTEMPTS,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RepairBudget {
    attempts: u8,
    limit: u8,
}

impl RepairBudget {
    pub(super) fn new(limit: u8) -> Self {
        Self { attempts: 0, limit }
    }

    pub(super) fn claim(&mut self) -> Result<u8, TransitionReason> {
        if self.attempts == self.limit {
            return Err(TransitionReason::RepairBudgetExhausted);
        }
        self.attempts += 1;
        Ok(self.attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table_is_deterministic_for_low_and_high_risk() {
        // low-risk + no expected evidence → Executor only
        let sufficient = coding_plan(RiskLevel::ReadOnly, &[], &[]);
        assert_eq!(sufficient.roles, vec![CognitiveRole::Executor]);
        assert_eq!(sufficient.uncertainty, UncertaintyLevel::Low);
        assert_eq!(
            sufficient.reason,
            TransitionReason::LowRiskEvidenceSufficient
        );

        // low-risk + non-empty expected evidence → Explorer + Executor
        let missing = coding_plan(RiskLevel::ReadOnly, &[], &["investigation receipt".into()]);
        assert_eq!(
            missing.roles,
            vec![CognitiveRole::Explorer, CognitiveRole::Executor]
        );
        assert_eq!(missing.uncertainty, UncertaintyLevel::Medium);

        let high = coding_plan(
            RiskLevel::SystemModify,
            &["crates/a".into()],
            &["test".into()],
        );
        assert_eq!(
            high.roles,
            vec![
                CognitiveRole::Planner,
                CognitiveRole::Explorer,
                CognitiveRole::Executor
            ]
        );
        assert_eq!(high.uncertainty, UncertaintyLevel::High);
    }

    #[test]
    fn acceptance_and_repair_budget_are_evidence_and_risk_driven() {
        let low = acceptance_plan(RiskLevel::ReadOnly, &[]);
        assert!(!low.tester_required);
        assert!(!low.reviewer_required);
        let high = acceptance_plan(RiskLevel::SystemModify, &[]);
        assert!(high.tester_required);
        assert!(high.reviewer_required);

        let mut budget = RepairBudget::new(2);
        assert_eq!(budget.claim(), Ok(1));
        assert_eq!(budget.claim(), Ok(2));
        assert_eq!(budget.claim(), Err(TransitionReason::RepairBudgetExhausted));
    }

    #[test]
    fn transition_reason_codes_are_stable_and_nonempty() {
        for reason in [
            TransitionReason::StructuredArtifactAccepted,
            TransitionReason::StructuredArtifactRejected,
            TransitionReason::ValidationRequired,
            TransitionReason::ValidationFailed,
            TransitionReason::ReviewRequired,
            TransitionReason::ReviewFailed,
            TransitionReason::StructuredFailureRepair,
            TransitionReason::RepairValidated,
            TransitionReason::RepairBudgetExhausted,
            TransitionReason::Cancelled,
        ] {
            assert!(!reason.code().is_empty());
            assert!(reason
                .code()
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_'));
        }
    }
}
