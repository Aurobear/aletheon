//! Host-owned engineering-task acceptance and repair projection.
//!
//! The model cannot call this controller or supply its decision. It consumes a
//! persisted deterministic Evaluation receipt, then updates the canonical
//! Agora task graph with the same finding identities before returning a public
//! Task settlement.

use crate::AgoraService;
use std::sync::Arc;

use ::contracts::cognitive_workflow::{
    CognitiveStage, CognitiveTaskNodeId, CognitiveTaskStatus, StageDecision, StageDecisionKind,
};
use ::contracts::{
    AgoraSpaceId, EvaluationDecision, EvaluationReceiptRef, ProcessId, TaskSettlement,
};

use crate::cognitive_workspace::CognitiveWorkspaceCoordinator;

pub struct HostAcceptanceController {
    agora: Arc<dyn AgoraService>,
    workspace: CognitiveWorkspaceCoordinator,
}

impl HostAcceptanceController {
    pub fn new(agora: Arc<dyn AgoraService>) -> Self {
        Self {
            workspace: CognitiveWorkspaceCoordinator::new(agora.clone()),
            agora,
        }
    }

    /// Apply an enforce-mode evaluation decision to the exact turn-root task.
    /// Shadow decisions remain observational and therefore return no terminal
    /// Task settlement.
    pub async fn settle_evaluation(
        &self,
        session_id: &str,
        receipt: &EvaluationReceiptRef,
        owner: ProcessId,
    ) -> anyhow::Result<Option<TaskSettlement>> {
        let settlement = match receipt.decision {
            EvaluationDecision::Accepted if receipt.failed_gates.is_empty() => {
                TaskSettlement::Accepted
            }
            EvaluationDecision::Accepted | EvaluationDecision::Rejected => {
                TaskSettlement::RepairRequired
            }
            EvaluationDecision::Indeterminate => TaskSettlement::Blocked,
            EvaluationDecision::ObservedPass | EvaluationDecision::ObservedFail => return Ok(None),
        };
        anyhow::ensure!(
            receipt.subject_kind == "turn",
            "engineering acceptance requires a turn evaluation subject"
        );
        let space = AgoraSpaceId(session_id.to_owned());
        let task_id = CognitiveTaskNodeId(format!("root:{}", receipt.subject_id));
        let task_list = self.agora.list_tasks(space.clone()).await?;
        let mut task = task_list
            .tasks
            .into_iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| anyhow::anyhow!("evaluation root task is absent from Agora"))?;
        let evidence_ref = format!("evaluation-receipt:{}", receipt.receipt_id.0);
        let finding_ids = receipt
            .failed_gates
            .iter()
            .enumerate()
            .map(|(index, _)| format!("evaluation:{}:{index}", receipt.receipt_id.0))
            .collect::<Vec<_>>();

        let (stage, status, decision, reason) = match settlement {
            TaskSettlement::Accepted => (
                CognitiveStage::Complete,
                CognitiveTaskStatus::Completed,
                StageDecisionKind::Accept,
                "host evaluation accepted all required gates",
            ),
            TaskSettlement::RepairRequired => (
                CognitiveStage::Validation,
                CognitiveTaskStatus::Running,
                StageDecisionKind::Repair,
                "host evaluation requires repair",
            ),
            TaskSettlement::Blocked => (
                CognitiveStage::Validation,
                CognitiveTaskStatus::Blocked,
                StageDecisionKind::Block,
                "host evaluation is indeterminate",
            ),
            _ => unreachable!("evaluation controller emits only acceptance gate settlements"),
        };
        task.stage = stage;
        task.status = status;
        task.unresolved_finding_ids = finding_ids.clone();
        let version = self
            .workspace
            .commit_task_at(space.clone(), task_list.workspace_version, task, owner)
            .await
            .map_err(anyhow::Error::new)?;
        self.workspace
            .record_stage_decision_at(
                space,
                version,
                task_id,
                StageDecision {
                    decision,
                    reason: reason.into(),
                    finding_ids,
                    evidence_refs: vec![evidence_ref],
                },
                owner,
            )
            .await
            .map_err(anyhow::Error::new)?;
        Ok(Some(settlement))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::cognitive_workflow::{CognitiveRole, CognitiveRoleProfile, CognitiveTaskNode};
    use ::contracts::{EvaluationContractId, EvaluationReceiptId};

    #[tokio::test]
    async fn u_verify_002_review_finding_modifies_the_canonical_plan_graph() {
        let clock: Arc<dyn ::contracts::Clock> = Arc::new(kernel::chronos::TestClock::new(1, 1));
        let agora = Arc::new(crate::AgoraRegistry::new(clock));
        let workspace = CognitiveWorkspaceCoordinator::new(agora.clone());
        let owner = ProcessId::new();
        let turn_id = ::contracts::TurnId::new();
        let task_id = CognitiveTaskNodeId(format!("root:{}", turn_id.0));
        let profile = CognitiveRoleProfile::canonical(CognitiveRole::Root);
        workspace
            .commit_task_at(
                AgoraSpaceId("session".into()),
                0,
                CognitiveTaskNode {
                    id: task_id.clone(),
                    parent_id: None,
                    objective: "verify the change".into(),
                    role: CognitiveRole::Root,
                    stage: CognitiveStage::Execution,
                    status: CognitiveTaskStatus::Running,
                    owner: Some(owner),
                    role_profile: profile.reference,
                    budget: profile.budget,
                    dependencies: vec![],
                    acceptance_criteria: vec!["tests pass".into()],
                    workspace_scope: vec!["/tmp".into()],
                    required_artifact_kinds: vec![],
                    artifact_refs: vec![],
                    unresolved_finding_ids: vec![],
                },
                owner,
            )
            .await
            .unwrap();
        let receipt_id = EvaluationReceiptId::new();
        let receipt = EvaluationReceiptRef {
            schema_version: ::contracts::EVALUATION_SCHEMA_V1,
            receipt_id,
            contract_id: EvaluationContractId::new(),
            subject_kind: "turn".into(),
            subject_id: turn_id.0.to_string(),
            decision: EvaluationDecision::Rejected,
            weighted_total_millis: Some(10),
            evidence_coverage_millis: 500,
            confidence_millis: 900,
            failed_gates: vec!["required_tests_passed".into()],
            created_at_ms: 1,
        };

        let settlement = HostAcceptanceController::new(agora.clone())
            .settle_evaluation("session", &receipt, owner)
            .await
            .unwrap();

        assert_eq!(settlement, Some(TaskSettlement::RepairRequired));
        let tasks = agora
            .list_tasks(AgoraSpaceId("session".into()))
            .await
            .unwrap();
        let task = tasks
            .tasks
            .into_iter()
            .find(|task| task.id == task_id)
            .unwrap();
        assert_eq!(task.stage, CognitiveStage::Validation);
        assert_eq!(task.status, CognitiveTaskStatus::Running);
        assert_eq!(
            task.unresolved_finding_ids,
            vec![format!("evaluation:{}:0", receipt_id.0)]
        );
    }
}
