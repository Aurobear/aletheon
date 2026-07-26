//! Executive coordinator for binding root work and role runtimes to one scoped
//! Agora task graph. It passes bounded artifact projections, never peer history.

use std::sync::Arc;

use fabric::cognitive_workflow::{
    AgoraProjectionRequest, AgoraTaskProjection, ClarificationId, ClarificationRecord,
    CognitiveArtifactKind, CognitiveInterruptionId, CognitiveInterruptionRecord, CognitiveRole,
    CognitiveTaskNode, CognitiveTaskNodeId,
};
use fabric::{
    AgoraOperation, AgoraProposal, AgoraService, AgoraSpaceId, ProcessId, WorkspaceCommitPermit,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgoraReconciliationRequired {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub proposed_version: u64,
    pub actual_version: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CognitiveWorkspaceError {
    #[error("Agora version conflict for task {0:?}; refresh the selected projection and re-plan")]
    Reconciliation(AgoraReconciliationRequired),
    #[error(transparent)]
    Service(#[from] anyhow::Error),
}

pub struct CognitiveWorkspaceCoordinator {
    agora: Arc<dyn AgoraService>,
}

impl CognitiveWorkspaceCoordinator {
    pub fn new(agora: Arc<dyn AgoraService>) -> Self {
        Self { agora }
    }

    /// The authenticated root session/Goal identity is the scope boundary. No
    /// machine-global mutable cognitive space is synthesized here.
    pub fn root_space(root_session_or_goal_id: impl Into<String>) -> AgoraSpaceId {
        AgoraSpaceId(root_session_or_goal_id.into())
    }

    pub async fn commit_task_at(
        &self,
        space: AgoraSpaceId,
        expected_version: u64,
        task: CognitiveTaskNode,
        author: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        let task_node_id = task.id.clone();
        self.commit_operation_at(
            space,
            expected_version,
            task_node_id,
            AgoraOperation::UpsertCognitiveTask { task },
            author,
        )
        .await
    }

    pub async fn block_for_clarification(
        &self,
        space: AgoraSpaceId,
        expected_version: u64,
        clarification: ClarificationRecord,
        owner: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        let task_node_id = clarification.task_node_id.clone();
        self.commit_operation_at(
            space,
            expected_version,
            task_node_id,
            AgoraOperation::BlockForClarification { clarification },
            owner,
        )
        .await
    }

    /// Apply a response only after Executive has appended the canonical user
    /// input event and can supply its stable identity.
    pub async fn resume_from_user_response(
        &self,
        space: AgoraSpaceId,
        task_node_id: CognitiveTaskNodeId,
        clarification_id: ClarificationId,
        response: String,
        response_event_id: String,
        expected_version: u64,
        owner: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        self.commit_operation_at(
            space,
            expected_version,
            task_node_id,
            AgoraOperation::ResolveClarification {
                clarification_id,
                response,
                response_event_id,
            },
            owner,
        )
        .await
    }

    pub async fn checkpoint_interruption(
        &self,
        space: AgoraSpaceId,
        expected_version: u64,
        interruption: CognitiveInterruptionRecord,
        owner: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        let task_node_id = interruption.task_node_id.clone();
        self.commit_operation_at(
            space,
            expected_version,
            task_node_id,
            AgoraOperation::CheckpointInterruption { interruption },
            owner,
        )
        .await
    }

    pub async fn resume_interruption(
        &self,
        space: AgoraSpaceId,
        task_node_id: CognitiveTaskNodeId,
        interruption_id: CognitiveInterruptionId,
        resume_event_id: String,
        expected_version: u64,
        owner: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        self.commit_operation_at(
            space,
            expected_version,
            task_node_id,
            AgoraOperation::ResumeInterruption {
                interruption_id,
                resume_event_id,
            },
            owner,
        )
        .await
    }

    async fn commit_operation_at(
        &self,
        space: AgoraSpaceId,
        expected_version: u64,
        task_node_id: CognitiveTaskNodeId,
        operation: AgoraOperation,
        author: ProcessId,
    ) -> Result<u64, CognitiveWorkspaceError> {
        let view = self
            .agora
            .view(fabric::AgoraViewRequest {
                space: space.clone(),
            })
            .await?;
        if view.version != expected_version {
            return Err(CognitiveWorkspaceError::Reconciliation(
                AgoraReconciliationRequired {
                    space,
                    task_node_id,
                    proposed_version: expected_version,
                    actual_version: view.version,
                },
            ));
        }
        let proposal = AgoraProposal {
            id: Uuid::new_v4(),
            space: space.clone(),
            author,
            base_version: expected_version,
            operation,
            evidence: Vec::new(),
            confidence: 1.0,
            expires_at_ms: None,
        };
        let permit = WorkspaceCommitPermit::issue_for(&proposal, i64::MAX)?;
        let proposal_id = match self.agora.propose(proposal).await {
            Ok(id) => id,
            Err(error) => {
                let actual_version = self
                    .agora
                    .view(fabric::AgoraViewRequest {
                        space: space.clone(),
                    })
                    .await?
                    .version;
                if actual_version != expected_version {
                    return Err(CognitiveWorkspaceError::Reconciliation(
                        AgoraReconciliationRequired {
                            space,
                            task_node_id,
                            proposed_version: expected_version,
                            actual_version,
                        },
                    ));
                }
                return Err(CognitiveWorkspaceError::Service(error));
            }
        };
        let receipt = self.agora.commit(proposal_id, permit).await?;
        Ok(receipt.commit.version)
    }

    pub async fn project_role(
        &self,
        space: AgoraSpaceId,
        task_node_id: CognitiveTaskNodeId,
        role: CognitiveRole,
        max_artifacts: usize,
        include_kinds: Vec<CognitiveArtifactKind>,
    ) -> anyhow::Result<AgoraTaskProjection> {
        self.agora
            .project_task(AgoraProjectionRequest {
                space,
                task_node_id,
                role,
                max_artifacts,
                include_kinds,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::cognitive_workflow::{CognitiveStage, CognitiveTaskStatus};

    fn task(id: &str, owner: ProcessId) -> CognitiveTaskNode {
        CognitiveTaskNode {
            id: CognitiveTaskNodeId(id.into()),
            parent_id: None,
            objective: "grounded work".into(),
            role: CognitiveRole::Planner,
            stage: CognitiveStage::Planning,
            status: CognitiveTaskStatus::Running,
            owner: Some(owner),
            dependencies: Vec::new(),
            acceptance_criteria: vec!["mapped requirements".into()],
            workspace_scope: vec!["crates/agora".into()],
            required_artifact_kinds: vec![CognitiveArtifactKind::Plan],
            artifact_refs: Vec::new(),
            unresolved_finding_ids: Vec::new(),
        }
    }

    #[tokio::test]
    async fn root_spaces_are_isolated_and_role_projection_is_receipted() {
        let agora: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )));
        let coordinator = CognitiveWorkspaceCoordinator::new(agora);
        let owner = ProcessId::new();
        coordinator
            .commit_task_at(
                CognitiveWorkspaceCoordinator::root_space("session-a"),
                0,
                task("plan", owner),
                owner,
            )
            .await
            .unwrap();
        let selected = coordinator
            .project_role(
                AgoraSpaceId("session-a".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                4,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(selected.receipt.space.0, "session-a");
        assert_eq!(selected.receipt.workspace_version, 1);
        assert_eq!(selected.receipt.role, CognitiveRole::Planner);
        assert!(coordinator
            .project_role(
                AgoraSpaceId("session-b".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                4,
                Vec::new(),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn stale_role_result_returns_typed_reconciliation() {
        let agora: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )));
        let coordinator = CognitiveWorkspaceCoordinator::new(agora);
        let owner = ProcessId::new();
        coordinator
            .commit_task_at(
                AgoraSpaceId("session".into()),
                0,
                task("plan", owner),
                owner,
            )
            .await
            .unwrap();
        let error = coordinator
            .commit_task_at(
                AgoraSpaceId("session".into()),
                0,
                task("plan", owner),
                owner,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            CognitiveWorkspaceError::Reconciliation(AgoraReconciliationRequired {
                proposed_version: 0,
                actual_version: 1,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn blocked_checkpoint_survives_restart_and_resumes_same_task() {
        use fabric::cognitive_workflow::{ClarificationState, CognitiveCheckpoint};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agora.db");
        let owner = ProcessId::new();
        let clarification_id = ClarificationId::new();
        {
            let persistence = Arc::new(agora::SqliteAgoraPersistence::open(&path).unwrap());
            let agora: Arc<dyn AgoraService> =
                Arc::new(agora::AgoraRegistry::new_with_persistence(
                    persistence,
                    Arc::new(kernel::chronos::TestClock::new(10, 0)),
                ));
            let coordinator = CognitiveWorkspaceCoordinator::new(agora);
            coordinator
                .commit_task_at(
                    AgoraSpaceId("durable-session".into()),
                    0,
                    task("plan", owner),
                    owner,
                )
                .await
                .unwrap();
            coordinator
                .block_for_clarification(
                    AgoraSpaceId("durable-session".into()),
                    1,
                    ClarificationRecord {
                        id: clarification_id.clone(),
                        task_node_id: CognitiveTaskNodeId("plan".into()),
                        requested_by: owner,
                        question: "Which contract is authoritative?".into(),
                        choices: vec!["A".into(), "B".into()],
                        checkpoint: CognitiveCheckpoint {
                            workspace_version: 1,
                            task_contract_artifact_ref: None,
                            outstanding_obligations: vec!["resolve ambiguity".into()],
                            validation_artifact_refs: Vec::new(),
                            runtime_receipt_refs: vec!["receipt://tool/1".into()],
                        },
                        state: ClarificationState::Pending,
                        response: None,
                        response_event_id: None,
                    },
                    owner,
                )
                .await
                .unwrap();
        }

        let persistence = Arc::new(agora::SqliteAgoraPersistence::open(&path).unwrap());
        let agora: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new_with_persistence(
            persistence,
            Arc::new(kernel::chronos::TestClock::new(20, 0)),
        ));
        let coordinator = CognitiveWorkspaceCoordinator::new(agora);
        let recovered = coordinator
            .project_role(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                8,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(recovered.workspace_version, 2);
        assert_eq!(recovered.task.status, CognitiveTaskStatus::Blocked);
        let pending = recovered.clarification.unwrap();
        assert_eq!(pending.id, clarification_id);
        assert_eq!(
            pending.checkpoint.runtime_receipt_refs,
            vec!["receipt://tool/1"]
        );

        coordinator
            .resume_from_user_response(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                clarification_id,
                "A is authoritative".into(),
                "session-item:42".into(),
                2,
                owner,
            )
            .await
            .unwrap();
        let resumed = coordinator
            .project_role(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                8,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(resumed.workspace_version, 3);
        assert_eq!(resumed.task.status, CognitiveTaskStatus::Running);
        assert!(resumed.clarification.is_none());

        use fabric::cognitive_workflow::{
            CognitiveInterruptionId, CognitiveInterruptionReason, CognitiveInterruptionRecord,
        };
        let interruption_id = CognitiveInterruptionId::new();
        coordinator
            .checkpoint_interruption(
                AgoraSpaceId("durable-session".into()),
                3,
                CognitiveInterruptionRecord {
                    id: interruption_id.clone(),
                    task_node_id: CognitiveTaskNodeId("plan".into()),
                    owner,
                    reason: CognitiveInterruptionReason::DaemonShutdown,
                    checkpoint: CognitiveCheckpoint {
                        workspace_version: 3,
                        task_contract_artifact_ref: None,
                        outstanding_obligations: vec!["finish accepted plan".into()],
                        validation_artifact_refs: Vec::new(),
                        runtime_receipt_refs: vec!["receipt://runtime/terminal".into()],
                    },
                    resume_event_id: None,
                },
                owner,
            )
            .await
            .unwrap();
        drop(coordinator);

        let persistence = Arc::new(agora::SqliteAgoraPersistence::open(&path).unwrap());
        let agora: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new_with_persistence(
            persistence,
            Arc::new(kernel::chronos::TestClock::new(30, 0)),
        ));
        let coordinator = CognitiveWorkspaceCoordinator::new(agora);
        let interrupted = coordinator
            .project_role(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                8,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(interrupted.workspace_version, 4);
        assert_eq!(interrupted.task.status, CognitiveTaskStatus::Suspended);
        assert_eq!(
            interrupted.interruption.as_ref().unwrap().id,
            interruption_id
        );
        coordinator
            .resume_interruption(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                interruption_id,
                "daemon-start:2".into(),
                4,
                owner,
            )
            .await
            .unwrap();
        let resumed_again = coordinator
            .project_role(
                AgoraSpaceId("durable-session".into()),
                CognitiveTaskNodeId("plan".into()),
                CognitiveRole::Planner,
                8,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(resumed_again.workspace_version, 5);
        assert_eq!(resumed_again.task.status, CognitiveTaskStatus::Running);
        assert!(resumed_again.interruption.is_none());
    }
}
