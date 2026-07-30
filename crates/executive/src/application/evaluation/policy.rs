use fabric::{EvaluationDecision, EvaluationMode, TurnStop};

#[derive(Debug, Default)]
pub struct EvaluationSettlementPolicy;

impl EvaluationSettlementPolicy {
    pub fn decision(
        mode: EvaluationMode,
        report: &fabric::types::metacognition_evaluation::EvaluationReport,
    ) -> EvaluationDecision {
        match (mode, report.eligible) {
            (EvaluationMode::Shadow, true) => EvaluationDecision::ObservedPass,
            (EvaluationMode::Shadow, false) => EvaluationDecision::ObservedFail,
            (EvaluationMode::Enforce, true) => EvaluationDecision::Accepted,
            (EvaluationMode::Enforce, false) => EvaluationDecision::Rejected,
        }
    }

    pub fn settle_stop(decision: EvaluationDecision, current: TurnStop) -> TurnStop {
        match decision {
            EvaluationDecision::Rejected | EvaluationDecision::Indeterminate => TurnStop::Blocked,
            EvaluationDecision::ObservedPass
            | EvaluationDecision::ObservedFail
            | EvaluationDecision::Accepted => current,
        }
    }
}
