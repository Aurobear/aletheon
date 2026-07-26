//! Executive coordinator for binding root work and role runtimes to one scoped
//! Agora task graph. It passes bounded artifact projections, never peer history.

use std::sync::Arc;

use fabric::cognitive_workflow::{
    AgoraProjectionRequest, AgoraTaskProjection, CognitiveArtifactKind, CognitiveRole,
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
            operation: AgoraOperation::UpsertCognitiveTask { task },
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
}
