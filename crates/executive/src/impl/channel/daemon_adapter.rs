//! Production turn executor that delegates to `DaemonTurnOrchestrator`.
//!
//! This adapter bridges the channel router's `ChannelTurnExecutor` trait
//! with the full daemon turn pipeline, extracting the assistant response
//! text from the JSON-RPC envelope returned by `execute_turn()`.

use std::sync::Arc;

use crate::r#impl::approval::ApplyCoordinator;
use crate::r#impl::approval::{ApprovalDecision, ApprovalRepository, ApprovalResolutionContext};
use crate::r#impl::channel::gmail::GmailGoalDraftCoordinator;
use crate::r#impl::goal::ObjectiveStore;
use crate::service::DaemonTurnOrchestrator;
use fabric::{
    ApprovalId, ApprovalSnapshot, GoalId, GoalSnapshot, GoalSpec, GoalState, PrincipalId, ProcessId,
};
use gateway::dispatcher::{ChannelGoalExecutor, ChannelTurnExecutor};
use gateway::ports::{ChannelApprovalDecision, ChannelApprovalPort};
use gateway::registry::ApprovalResolver;
use tokio::sync::Mutex;

/// Wraps a `DaemonTurnOrchestrator` so the channel router can invoke
/// full daemon chat turns without depending on the handler stack.
pub struct DaemonChannelTurnExecutor {
    orchestrator: Arc<DaemonTurnOrchestrator>,
}

pub struct DaemonChannelGoalExecutor {
    store: Arc<Mutex<ObjectiveStore>>,
}

pub struct DaemonChannelApprovalExecutor {
    coordinator: Arc<ApplyCoordinator>,
    owner_process: ProcessId,
    cancel: tokio_util::sync::CancellationToken,
}

pub struct DaemonGmailDraftApprovalExecutor {
    coordinator: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>,
}

impl DaemonGmailDraftApprovalExecutor {
    pub fn new(coordinator: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>) -> Self {
        Self { coordinator }
    }
}

#[async_trait::async_trait]
impl ApprovalResolver for DaemonGmailDraftApprovalExecutor {
    async fn execute_resolved(
        &self,
        approval: &fabric::ApprovalSnapshot,
        action: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let coordinator = self.coordinator.lock().unwrap();
        match action {
            "confirm" => {
                coordinator.confirm(approval, now_ms)?;
            }
            "edit" => {
                coordinator.reject_or_edit(approval, true, now_ms)?;
            }
            "reject" => {
                coordinator.reject_or_edit(approval, false, now_ms)?;
            }
            _ => anyhow::bail!("unsupported Gmail draft approval action"),
        }
        Ok(())
    }

    async fn revise_draft(
        &self,
        owner: &str,
        goal_id: GoalId,
        intent: &str,
        now_ms: i64,
    ) -> anyhow::Result<fabric::ApprovalSnapshot> {
        self.coordinator
            .lock()
            .unwrap()
            .revise(
                goal_id,
                &PrincipalId(owner.to_owned()),
                intent,
                now_ms,
                now_ms.saturating_add(24 * 60 * 60 * 1_000),
            )
            .map(|draft| draft.approval)
    }
}

impl DaemonChannelApprovalExecutor {
    pub fn new(
        coordinator: Arc<ApplyCoordinator>,
        owner_process: ProcessId,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            coordinator,
            owner_process,
            cancel,
        }
    }
}

#[async_trait::async_trait]
impl ApprovalResolver for DaemonChannelApprovalExecutor {
    async fn execute_resolved(
        &self,
        approval: &fabric::ApprovalSnapshot,
        _action: &str,
        _now_ms: i64,
    ) -> anyhow::Result<()> {
        self.coordinator
            .coordinate(approval.id, self.owner_process, self.cancel.child_token())
            .await?;
        Ok(())
    }
}

impl DaemonChannelGoalExecutor {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }

    fn ensure_owner(goal: GoalSnapshot, owner: &str) -> anyhow::Result<GoalSnapshot> {
        if goal.owner.0 != owner {
            anyhow::bail!("goal not found");
        }
        Ok(goal)
    }
}

#[async_trait::async_trait]
impl ChannelGoalExecutor for DaemonChannelGoalExecutor {
    async fn create_draft(&self, owner: &str, intent: &str) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        if store
            .list_goals(&[], 100)?
            .into_iter()
            .any(|g| !g.state.is_terminal())
        {
            anyhow::bail!("an active goal already exists");
        }
        let spec = GoalSpec {
            original_intent: intent.into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        store.create_draft_goal(&PrincipalId(owner.into()), owner, "session", &spec)
    }

    async fn list(&self, owner: &str) -> anyhow::Result<Vec<GoalSnapshot>> {
        Ok(self
            .store
            .lock()
            .await
            .list_goals(&[], 100)?
            .into_iter()
            .filter(|g| g.owner.0 == owner)
            .collect())
    }

    async fn show(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let goal = self
            .store
            .lock()
            .await
            .get_goal(id)?
            .ok_or_else(|| anyhow::anyhow!("goal not found"))?;
        Self::ensure_owner(goal, owner)
    }

    async fn pause(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Suspended,
            None,
            &serde_json::json!({"action":"pause"}),
        )?)
    }

    async fn resume(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Ready,
            None,
            &serde_json::json!({"action":"resume"}),
        )?)
    }

    async fn cancel(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot> {
        let store = self.store.lock().await;
        let goal = Self::ensure_owner(
            store
                .get_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("goal not found"))?,
            owner,
        )?;
        Ok(store.transition_goal(
            id,
            goal.version,
            GoalState::Cancelled,
            None,
            &serde_json::json!({"action":"cancel"}),
        )?)
    }
}

impl DaemonChannelTurnExecutor {
    pub fn new(orchestrator: Arc<DaemonTurnOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait::async_trait]
impl ChannelTurnExecutor for DaemonChannelTurnExecutor {
    async fn execute(
        &self,
        principal: &str,
        message: &str,
        correlation_id: &str,
    ) -> anyhow::Result<String> {
        let resp = self
            .orchestrator
            .execute_authenticated_turn(
                serde_json::Value::String(correlation_id.to_string()),
                message,
                fabric::PrincipalContext::new(
                    PrincipalId::local_uid(nix::unistd::Uid::effective().as_raw()),
                    fabric::LocalOsPrincipal {
                        uid: nix::unistd::Uid::effective().as_raw(),
                        gid: nix::unistd::Gid::effective().as_raw(),
                    },
                    fabric::ConnectionId::new(),
                    fabric::ThreadId(principal.to_owned()),
                    fabric::WorkspacePolicy::from_resolved_roots(
                        std::path::PathBuf::from("/var/lib/aletheon"),
                        Vec::new(),
                    )
                    .map_err(anyhow::Error::msg)?,
                    fabric::PermissionProfileId::workspace_write(),
                    fabric::ApprovalPolicy::OnRequest,
                ),
            )
            .await;

        // Success shape:
        //   {"jsonrpc": "2.0", "id": ..., "result": {"response": "...", "turn": N}}
        // Error shape:
        //   {"jsonrpc": "2.0", "id": ..., "error": {"code": ..., "message": "..."}}
        if let Some(err) = resp.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown turn error");
            anyhow::bail!("turn failed: {msg}");
        }

        let text = resp
            .get("result")
            .and_then(|r| r.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or("");
        Ok(text.to_string())
    }
}

/// Adapts the concrete `ApprovalRepository` to the fabric-native
/// [`ChannelApprovalPort`] so `dispatcher.rs` and `handlers/approval.rs`
/// no longer depend on `crate::r#impl::approval::*` directly.
pub struct ApprovalRepositoryPort {
    repository: Arc<std::sync::Mutex<ApprovalRepository>>,
}

impl ApprovalRepositoryPort {
    pub fn new(repository: Arc<std::sync::Mutex<ApprovalRepository>>) -> Self {
        Self { repository }
    }
}

impl ChannelApprovalPort for ApprovalRepositoryPort {
    fn get(&self, id: ApprovalId) -> anyhow::Result<Option<ApprovalSnapshot>> {
        Ok(self.repository.lock().unwrap().get(id)?)
    }

    fn resolve(
        &self,
        id: ApprovalId,
        expected_version: u64,
        principal: PrincipalId,
        channel: String,
        decision: ChannelApprovalDecision,
        now_ms: i64,
    ) -> anyhow::Result<ApprovalSnapshot> {
        let context = ApprovalResolutionContext {
            principal_id: principal,
            channel,
        };
        let decision = match decision {
            ChannelApprovalDecision::Approve => ApprovalDecision::Approve,
            ChannelApprovalDecision::Reject { reason } => ApprovalDecision::Reject { reason },
        };
        Ok(self.repository.lock().unwrap().resolve(
            id,
            expected_version,
            &context,
            decision,
            now_ms,
        )?)
    }

    fn record_delivery_pending(
        &self,
        approval_id: ApprovalId,
        channel: &str,
        conversation_id: &str,
        correlation_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.repository.lock().unwrap().record_delivery_pending(
            approval_id,
            channel,
            conversation_id,
            correlation_id,
            now_ms,
        )?;
        Ok(())
    }

    fn record_delivery_sent(
        &self,
        correlation_id: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        Ok(self.repository.lock().unwrap().record_delivery_sent(
            correlation_id,
            provider_message_id,
            now_ms,
        )?)
    }

    fn record_delivery_failed(
        &self,
        correlation_id: &str,
        error: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        Ok(self
            .repository
            .lock()
            .unwrap()
            .record_delivery_failed(correlation_id, error, now_ms)?)
    }

    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> anyhow::Result<Vec<ApprovalSnapshot>> {
        Ok(self
            .repository
            .lock()
            .unwrap()
            .list_pending(principal, now_ms)?)
    }
}
