//! M1 evidence-driven multi-agent controller (Aletheon closure plan §17).
//!
//! Replaces the fixed Planner→Explorer→Executor role pipeline with a private
//! stage machine that can skip stages (low risk), run independent exploration
//! in bounded parallel (shared writes stay serialized/isolated), and trigger
//! Tester/Reviewer on missing evidence rather than fixed role counts.  Fixer
//! only receives structured failure evidence with a loop cap.  All roles share
//! one OperationScope, one permission context and one terminal settlement —
//! there is still exactly one Turn Engine and one settlement path.  No new
//! top-level crate; this is a seam, not a second engine.

/// Task risk/uncertainty classification (drives stage skipping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRisk {
    Low,
    Medium,
    High,
}

/// Missing-evidence trigger for Tester/Reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceGap {
    NoVerification,
    NoReview,
    NoRepro,
    None,
}

/// A role stage in the orchestration machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Planner,
    Explorer,
    Executor,
    Tester,
    Reviewer,
    Fixer,
}

/// Why a transition occurred (acceptance: "所有转换有原因码").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionReason {
    RiskLowSkip,
    EvidenceGap,
    FailureEvidence,
    LoopCapReached,
    Complete,
}

/// Evidence-driven orchestration decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestrationStep {
    Run(Stage),
    Skip(Stage),
    Settle,
}

/// The evidence-driven orchestration machine.  Pure: it never executes roles,
/// only decides.  All roles share one scope/permission/terminal (the caller
/// supplies the single `PerTurnScope` and settles via the one reducer).
pub struct EvidenceDrivenController;

impl EvidenceDrivenController {
    /// Decide the next stage for a task, or settle.
    pub fn next(
        &self,
        risk: TaskRisk,
        gap: EvidenceGap,
        fix_loop_count: u64,
        fix_loop_cap: u64,
    ) -> (OrchestrationStep, TransitionReason) {
        match gap {
            EvidenceGap::NoVerification => (
                OrchestrationStep::Run(Stage::Tester),
                TransitionReason::EvidenceGap,
            ),
            EvidenceGap::NoReview => (
                OrchestrationStep::Run(Stage::Reviewer),
                TransitionReason::EvidenceGap,
            ),
            EvidenceGap::NoRepro => {
                if fix_loop_count >= fix_loop_cap {
                    (OrchestrationStep::Settle, TransitionReason::LoopCapReached)
                } else {
                    (
                        OrchestrationStep::Run(Stage::Fixer),
                        TransitionReason::FailureEvidence,
                    )
                }
            }
            EvidenceGap::None => match risk {
                // Low risk skips Planner/Explorer entirely.
                TaskRisk::Low => (
                    OrchestrationStep::Skip(Stage::Planner),
                    TransitionReason::RiskLowSkip,
                ),
                TaskRisk::Medium => (
                    OrchestrationStep::Run(Stage::Explorer),
                    TransitionReason::EvidenceGap,
                ),
                TaskRisk::High => (
                    OrchestrationStep::Run(Stage::Planner),
                    TransitionReason::EvidenceGap,
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_risk_skips_planner() {
        let ctl = EvidenceDrivenController;
        let (step, reason) = ctl.next(TaskRisk::Low, EvidenceGap::None, 0, 3);
        assert_eq!(step, OrchestrationStep::Skip(Stage::Planner));
        assert_eq!(reason, TransitionReason::RiskLowSkip);
    }

    #[test]
    fn tester_reviewer_triggered_by_evidence_gap_not_fixed_count() {
        let ctl = EvidenceDrivenController;
        let (step, _) = ctl.next(TaskRisk::Medium, EvidenceGap::NoVerification, 0, 3);
        assert_eq!(step, OrchestrationStep::Run(Stage::Tester));
        let (step, _) = ctl.next(TaskRisk::Medium, EvidenceGap::NoReview, 0, 3);
        assert_eq!(step, OrchestrationStep::Run(Stage::Reviewer));
    }

    #[test]
    fn fixer_is_capped() {
        let ctl = EvidenceDrivenController;
        let (step, reason) = ctl.next(TaskRisk::High, EvidenceGap::NoRepro, 3, 3);
        assert_eq!(step, OrchestrationStep::Settle);
        assert_eq!(reason, TransitionReason::LoopCapReached);
    }
}
