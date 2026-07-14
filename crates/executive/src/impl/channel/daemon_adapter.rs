//! Production turn executor that delegates to `DaemonTurnOrchestrator`.
//!
//! This adapter bridges the channel router's `ChannelTurnExecutor` trait
//! with the full daemon turn pipeline, extracting the assistant response
//! text from the JSON-RPC envelope returned by `execute_turn()`.

use std::sync::Arc;

use crate::r#impl::approval::ApplyCoordinator;
use crate::r#impl::channel::router::{
    ChannelApprovalExecutor, ChannelGoalExecutor, ChannelTurnExecutor,
};
use crate::r#impl::goal::ObjectiveStore;
use crate::service::DaemonTurnOrchestrator;
use fabric::{ApprovalId, GoalId, GoalSnapshot, GoalSpec, GoalState, PrincipalId, ProcessId};
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
impl ChannelApprovalExecutor for DaemonChannelApprovalExecutor {
    async fn execute_resolved(&self, approval_id: ApprovalId) -> anyhow::Result<()> {
        self.coordinator
            .coordinate(approval_id, self.owner_process, self.cancel.child_token())
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
    async fn execute(&self, message: &str, correlation_id: &str) -> anyhow::Result<String> {
        let resp = self
            .orchestrator
            .execute_turn(
                serde_json::Value::String(correlation_id.to_string()),
                message,
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
