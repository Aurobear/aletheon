//! SQLite adapters for approved-apply persistence and Goal settlement.

use crate::{approval_repository::ApprovalRepository, goal::ObjectiveStore};
use application::approval::{
    ApprovalApplyClaim, ApprovalApplyReceipt, ApprovalApplyRepositoryPort, ApprovedApplyGoalPort,
};
use contracts::{ApprovalId, ApprovalSnapshot, GoalId, GoalState, GoalWaitReason, OperationId};
use std::sync::{Arc, Mutex};

pub struct SqliteApprovalApplyRepository {
    repository: Arc<Mutex<ApprovalRepository>>,
}

impl SqliteApprovalApplyRepository {
    pub fn new(repository: Arc<Mutex<ApprovalRepository>>) -> Self {
        Self { repository }
    }
}

impl ApprovalApplyRepositoryPort for SqliteApprovalApplyRepository {
    fn approval(&self, id: ApprovalId) -> Result<Option<ApprovalSnapshot>, String> {
        self.repository
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .get(id)
            .map_err(|error| error.to_string())
    }
    fn receipt(&self, id: ApprovalId) -> Result<Option<ApprovalApplyReceipt>, String> {
        self.repository
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .apply_receipt(id)
            .map_err(|error| error.to_string())
    }
    fn claim(
        &self,
        id: ApprovalId,
        operation_id: OperationId,
        now_ms: i64,
    ) -> Result<ApprovalApplyClaim, String> {
        self.repository
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .claim_apply(id, operation_id, now_ms)
            .map_err(|error| error.to_string())
    }
    fn finish(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        self.repository
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .finish_apply(receipt)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

pub struct SqliteApprovedApplyGoal {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl SqliteApprovedApplyGoal {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }

    fn goal(&self, id: GoalId) -> Result<contracts::GoalSnapshot, String> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_goal(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "goal not found".to_string())
    }
}

impl ApprovedApplyGoalPort for SqliteApprovedApplyGoal {
    fn settle_rejected(
        &self,
        approval: &ApprovalSnapshot,
        revision_requested: bool,
    ) -> Result<(), String> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let goal = store
            .get_goal(approval.subject.goal_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "goal not found".to_string())?;
        let target = if revision_requested {
            GoalState::Ready
        } else {
            GoalState::Cancelled
        };
        if !goal.state.is_terminal() && goal.state != target {
            store
                .transition_goal(
                    goal.id,
                    goal.version,
                    target,
                    None,
                    &serde_json::json!({"approval_id":approval.id.0,"revision":revision_requested}),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn ensure_running(&self, goal_id: GoalId) -> Result<(), String> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let mut goal = store
            .get_goal(goal_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "goal not found".to_string())?;
        if matches!(
            goal.state,
            GoalState::AwaitingHuman | GoalState::Blocked | GoalState::Suspended
        ) {
            goal = store
                .transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Ready,
                    None,
                    &serde_json::json!({"action":"approved_apply_ready"}),
                )
                .map_err(|error| error.to_string())?;
        }
        if goal.state == GoalState::Ready {
            goal = store
                .transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Running,
                    None,
                    &serde_json::json!({"action":"approved_apply_running"}),
                )
                .map_err(|error| error.to_string())?;
        }
        if goal.state != GoalState::Running {
            return Err(format!("goal is not runnable: {}", goal.state));
        }
        Ok(())
    }

    fn settle_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let goal = store
            .get_goal(receipt.goal_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "goal not found".to_string())?;
        let target = if receipt.success {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        if goal.state == target {
            return Ok(());
        }
        let wait = (!receipt.success).then(|| GoalWaitReason::HumanInput {
            prompt: "Approved patch failed to apply; fresh verification and approval required"
                .into(),
        });
        store
            .transition_goal(
                goal.id,
                goal.version,
                target,
                wait.as_ref(),
                &serde_json::json!({"apply_receipt":receipt}),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn reconcile_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        let target = if receipt.success {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        if self.goal(receipt.goal_id)?.state != target {
            self.ensure_running(receipt.goal_id)?;
            self.settle_terminal(receipt)?;
        }
        Ok(())
    }
}
