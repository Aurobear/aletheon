//! Bounded GoalCoordinator — one tick per explicit call.
//!
//! M2 does NOT loop. Each `tick()` produces at most one transition or
//! one turn request. The caller (M3 worker) is responsible for scheduling
//! subsequent ticks.

use crate::r#impl::approval::ApprovalRepository;
use crate::r#impl::approval::{ApplyCoordinator, ApplyCoordinatorConfig, ManagedWorktreeCleaner};
use crate::r#impl::goal::budget::GoalBudgetRequest;
use crate::r#impl::goal::transition::GoalTransitionError;
use crate::r#impl::goal::{
    AttemptCoordinationOutcome, AttemptCoordinator, AttemptCoordinatorError, AttemptExecutor,
    AttemptRequest, CodingVerifier, ObjectiveStore, RetryPolicy,
};
use crate::r#impl::memory_projection::{
    ApprovedArchitectureDecision, MemoryProjection, ProjectionStatus,
};
use crate::r#impl::storage_quota::{StorageClass, StorageQuota, StorageReservation};
use aletheon_kernel::KernelRuntime;
use fabric::goal::{GoalId, GoalSnapshot, GoalState, GoalWaitReason};
use fabric::Clock;
use fabric::ProcessId;
use fabric::{ApprovalId, ExternalEventEnvelope, ExternalEventId, ExternalIdentityId, PrincipalId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// GoalTickOutcome
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum GoalTickOutcome {
    Noop { state: GoalState },
    Transitioned { from: GoalState, to: GoalState },
    TurnRequested { goal_id: GoalId, input: String },
    BudgetBlocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleEventWaitCondition {
    pub account_id: ExternalIdentityId,
    pub event_id: Option<ExternalEventId>,
    pub object_id: Option<String>,
    pub source_after_ms: Option<i64>,
    pub source_before_ms: Option<i64>,
}

impl GoogleEventWaitCondition {
    pub fn key(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    fn matches(&self, event: &ExternalEventEnvelope) -> bool {
        self.account_id == event.account_id
            && self.event_id.is_none_or(|id| id == event.id)
            && self
                .object_id
                .as_deref()
                .is_none_or(|id| id == event.object.object_id)
            && self
                .source_after_ms
                .is_none_or(|after| event.source_timestamp_ms >= after)
            && self
                .source_before_ms
                .is_none_or(|before| event.source_timestamp_ms <= before)
    }
}

// ---------------------------------------------------------------------------
// GoalCoordinator
// ---------------------------------------------------------------------------

pub struct GoalCoordinator {
    store: Arc<Mutex<ObjectiveStore>>,
    memory_projection: Option<MemoryProjection>,
    storage_quota: Option<(StorageQuota, u64)>,
    storage_reservations: Mutex<HashMap<GoalId, StorageReservation>>,
}

impl GoalCoordinator {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self {
            store,
            memory_projection: None,
            storage_quota: None,
            storage_reservations: Mutex::new(HashMap::new()),
        }
    }

    /// Reserve deployment storage for a Goal attempt before scheduling it.
    pub fn with_storage_quota(mut self, quota: StorageQuota, expected_attempt_bytes: u64) -> Self {
        self.storage_quota = Some((quota, expected_attempt_bytes));
        self
    }

    pub fn with_memory_projection(mut self, projection: MemoryProjection) -> Self {
        self.memory_projection = Some(projection);
        self
    }

    /// Reserve production storage before a daemon scheduler starts an attempt.
    pub fn admit_attempt_storage(&self, goal_id: GoalId) -> Result<(), String> {
        let Some((quota, expected_bytes)) = &self.storage_quota else {
            return Ok(());
        };
        let mut reservations = self.storage_reservations.lock().unwrap();
        if let std::collections::hash_map::Entry::Vacant(entry) = reservations.entry(goal_id) {
            let reservation = quota
                .reserve(StorageClass::Total, *expected_bytes, 1)
                .map_err(|error| error.to_string())?;
            entry.insert(reservation);
        }
        Ok(())
    }

    pub fn release_attempt_storage(&self, goal_id: GoalId) {
        self.storage_reservations.lock().unwrap().remove(&goal_id);
    }

    /// Project only an immutable summary that can be read back from durable
    /// storage. Failure is health evidence and never rewrites the Goal result.
    pub async fn project_persisted_goal_summary(
        &self,
        approval_id: ApprovalId,
        sensitivity: mnemosyne::MemorySensitivity,
    ) -> Option<ProjectionStatus> {
        let projection = self.memory_projection.as_ref()?;
        let loaded = {
            let store = self.store.lock().unwrap();
            let summary = store
                .load_goal_completion_summary(approval_id)
                .ok()
                .flatten()?;
            let evidence = store.goal_projection_evidence(summary.goal_id).ok()?;
            (summary, evidence)
        };
        Some(
            projection
                .project_goal_summary(&loaded.0, &loaded.1, sensitivity)
                .await,
        )
    }

    pub async fn project_approved_architecture_decision(
        &self,
        decision: &ApprovedArchitectureDecision,
    ) -> Option<ProjectionStatus> {
        let projection = self.memory_projection.as_ref()?;
        Some(projection.project_architecture_decision(decision).await)
    }

    /// Build the M3 one-shot attempt coordinator over this Goal store.
    /// Scheduling remains outside both coordinators, so one tick cannot loop
    /// into a second provider invocation.
    pub fn attempt_coordinator(
        &self,
        executor: Arc<dyn AttemptExecutor>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
    ) -> AttemptCoordinator {
        AttemptCoordinator::new(self.store.clone(), executor, clock, retry_policy)
    }

    /// Build a one-shot coordinator with the mandatory Pi verification gate.
    pub fn coding_attempt_coordinator(
        &self,
        executor: Arc<dyn AttemptExecutor>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
        verifier: Arc<dyn CodingVerifier>,
        worktree_base: impl AsRef<Path>,
    ) -> Result<AttemptCoordinator, AttemptCoordinatorError> {
        AttemptCoordinator::new(self.store.clone(), executor, clock, retry_policy)
            .with_coding_verification(verifier, worktree_base)
    }

    pub fn approval_coding_attempt_coordinator(
        &self,
        executor: Arc<dyn AttemptExecutor>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
        verifier: Arc<dyn CodingVerifier>,
        worktree_base: impl AsRef<Path>,
        approvals: Arc<Mutex<ApprovalRepository>>,
    ) -> Result<AttemptCoordinator, AttemptCoordinatorError> {
        self.coding_attempt_coordinator(executor, clock, retry_policy, verifier, worktree_base)?
            .with_approval_repository(approvals)
    }

    pub fn approved_apply_coordinator(
        &self,
        approvals: Arc<Mutex<ApprovalRepository>>,
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        config: ApplyCoordinatorConfig,
        cleaner: Arc<dyn ManagedWorktreeCleaner>,
    ) -> Result<ApplyCoordinator, crate::r#impl::approval::ApplyCoordinationError> {
        let coordinator = ApplyCoordinator::new(
            self.store.clone(),
            approvals,
            kernel,
            clock,
            config,
            cleaner,
        )?;
        Ok(match &self.memory_projection {
            Some(projection) => coordinator.with_memory_projection(projection.clone()),
            None => coordinator,
        })
    }

    /// Schedule exactly one durable runtime attempt for a Running Goal.
    /// A retry decision is persisted for a future tick; this method never loops.
    pub async fn tick_attempt(
        &self,
        attempt_coordinator: &AttemptCoordinator,
        request: AttemptRequest,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        attempt_coordinator.execute_one(request, cancel).await
    }

    /// Advance a goal by one bounded step.
    ///
    /// Policy:
    /// - terminal / draft / suspended / awaiting-human / blocked → Noop
    /// - ready → transition to Running
    /// - running → reserve one attempt → TurnRequested
    pub fn tick(
        &self,
        goal_id: GoalId,
        now_ms: i64,
    ) -> Result<GoalTickOutcome, GoalTransitionError> {
        let store = self.store.lock().unwrap();

        let current = match store.get_goal(goal_id)? {
            Some(g) => g,
            None => {
                return Ok(GoalTickOutcome::Noop {
                    state: GoalState::Cancelled,
                })
            }
        };

        match current.state {
            GoalState::Completed | GoalState::Failed | GoalState::Cancelled => {
                self.storage_reservations.lock().unwrap().remove(&goal_id);
                Ok(GoalTickOutcome::Noop {
                    state: current.state,
                })
            }
            GoalState::Draft => Ok(GoalTickOutcome::Noop {
                state: GoalState::Draft,
            }),
            GoalState::Suspended => Ok(GoalTickOutcome::Noop {
                state: GoalState::Suspended,
            }),
            GoalState::AwaitingHuman => Ok(GoalTickOutcome::Noop {
                state: GoalState::AwaitingHuman,
            }),
            GoalState::Blocked => Ok(GoalTickOutcome::Noop {
                state: GoalState::Blocked,
            }),
            GoalState::Ready => {
                let from = current.state;
                let snapshot = store.transition_goal(
                    goal_id,
                    current.version,
                    GoalState::Running,
                    None,
                    &serde_json::json!({"action": "tick_start"}),
                )?;
                Ok(GoalTickOutcome::Transitioned {
                    from,
                    to: snapshot.state,
                })
            }
            GoalState::Running => {
                if let Err(reason) = self.admit_attempt_storage(goal_id) {
                    return Ok(GoalTickOutcome::BudgetBlocked { reason });
                }
                let budget_res = store.reserve_goal_budget(
                    goal_id,
                    GoalBudgetRequest {
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0,
                        attempts: 1,
                    },
                    now_ms,
                );

                match budget_res {
                    Ok(_reservation) => {
                        let input =
                            format!("Goal #{}: {}", goal_id.0, current.spec.original_intent);
                        Ok(GoalTickOutcome::TurnRequested { goal_id, input })
                    }
                    Err(e) => Ok(GoalTickOutcome::BudgetBlocked {
                        reason: e.to_string(),
                    }),
                }
            }
        }
    }

    /// Wake only goals with an explicit persisted external-event condition.
    pub fn wake_for_google_event(
        &self,
        principal: &PrincipalId,
        event: &ExternalEventEnvelope,
    ) -> Result<Vec<GoalId>, GoalTransitionError> {
        let store = self.store.lock().unwrap();
        let goals = store.list_goals(
            &[
                GoalState::AwaitingHuman,
                GoalState::Suspended,
                GoalState::Blocked,
            ],
            100,
        )?;
        let mut woken = Vec::new();
        for goal in goals {
            if &goal.owner != principal || goal.state.is_terminal() {
                continue;
            }
            let Some(GoalWaitReason::ExternalEvent { key }) = &goal.wait_reason else {
                continue;
            };
            let Ok(condition) = serde_json::from_str::<GoogleEventWaitCondition>(key) else {
                continue;
            };
            if !condition.matches(event) {
                continue;
            }
            store.transition_goal(
                goal.id,
                goal.version,
                GoalState::Ready,
                None,
                &serde_json::json!({
                    "action":"google_external_event_wake",
                    "event_id":event.id.to_string(),
                    "object_id":event.object.object_id,
                }),
            )?;
            woken.push(goal.id);
        }
        Ok(woken)
    }

    /// Set the process link for a goal (atomic versioned event).
    pub fn set_process_link(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        process_id: Option<ProcessId>,
    ) -> Result<GoalSnapshot, GoalTransitionError> {
        let store = self.store.lock().unwrap();

        let current = match store.get_goal(goal_id)? {
            Some(g) => g,
            None => return Err(GoalTransitionError::NotFound(goal_id)),
        };

        if current.version != expected_version {
            return Err(GoalTransitionError::VersionConflict {
                expected: expected_version,
                actual: current.version,
            });
        }

        let new_version = current.version + 1;
        let process_id_str = process_id.map(|p| p.0.to_string());

        let tx = store.db.unchecked_transaction()?;

        let changed = tx.execute(
            "UPDATE objectives SET process_id = ?1, version = ?2,
             updated_at = datetime('now')
             WHERE objective_id = ?3 AND version = ?4",
            rusqlite::params![process_id_str, new_version, goal_id.0, expected_version],
        )?;

        if changed == 0 {
            tx.rollback().ok();
            let fresh = store.get_goal(goal_id)?;
            return match fresh {
                Some(g) => Err(GoalTransitionError::VersionConflict {
                    expected: expected_version,
                    actual: g.version,
                }),
                None => Err(GoalTransitionError::NotFound(goal_id)),
            };
        }

        tx.execute(
            "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
             VALUES (?1, ?2, 'process_link', ?3)",
            rusqlite::params![
                goal_id.0,
                new_version,
                serde_json::json!({"process_id": process_id_str}).to_string(),
            ],
        )?;

        tx.commit()?;

        store
            .get_goal(goal_id)?
            .ok_or(GoalTransitionError::NotFound(goal_id))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::goal::{GoalBudget, GoalSpec};
    use fabric::PrincipalId;
    use fabric::ProcessId;
    use tempfile::NamedTempFile;

    fn setup() -> (GoalCoordinator, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(tmp.path()).unwrap();
        let coordinator = GoalCoordinator::new(Arc::new(Mutex::new(store)));
        (coordinator, tmp)
    }

    fn create_goal(coord: &GoalCoordinator) -> GoalSnapshot {
        let store = coord.store.lock().unwrap();
        let spec = GoalSpec {
            original_intent: "tick test goal".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: GoalBudget {
                max_input_tokens: 100000,
                max_output_tokens: 50000,
                max_cost_usd: None,
                max_attempts: 3,
                deadline_ms: None,
            },
        };
        store
            .create_goal(&PrincipalId("test".into()), "s", "session", &spec)
            .unwrap()
    }

    #[test]
    fn tick_ready_transitions_to_running() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        let outcome = coord.tick(g.id, 0).unwrap();
        match outcome {
            GoalTickOutcome::Transitioned { from, to } => {
                assert_eq!(from, GoalState::Ready);
                assert_eq!(to, GoalState::Running);
            }
            _ => panic!("expected Transitioned, got {outcome:?}"),
        }
    }

    #[test]
    fn tick_running_requests_turn() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        coord.tick(g.id, 0).unwrap();
        let outcome = coord.tick(g.id, 0).unwrap();
        match outcome {
            GoalTickOutcome::TurnRequested { goal_id, .. } => {
                assert_eq!(goal_id, g.id);
            }
            _ => panic!("expected TurnRequested, got {outcome:?}"),
        }
    }

    #[test]
    fn tick_draft_is_noop() {
        let (coord, _tmp) = setup();
        let store = coord.store.lock().unwrap();
        let spec = GoalSpec {
            original_intent: "draft goal".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: GoalBudget::default(),
        };
        let g = store
            .create_goal(&PrincipalId("test".into()), "s", "session", &spec)
            .unwrap();
        drop(store);

        let store = coord.store.lock().unwrap();
        store
            .db
            .execute(
                "UPDATE objectives SET goal_state = 'draft' WHERE objective_id = ?1",
                rusqlite::params![g.id.0],
            )
            .unwrap();
        drop(store);

        let outcome = coord.tick(g.id, 0).unwrap();
        match outcome {
            GoalTickOutcome::Noop { state } => assert_eq!(state, GoalState::Draft),
            _ => panic!("expected Noop for Draft, got {outcome:?}"),
        }
    }

    #[test]
    fn tick_terminal_is_noop() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        coord.tick(g.id, 0).unwrap();
        let store = coord.store.lock().unwrap();
        store
            .transition_goal(g.id, 1, GoalState::Completed, None, &serde_json::json!({}))
            .unwrap();
        drop(store);

        let outcome = coord.tick(g.id, 0).unwrap();
        match outcome {
            GoalTickOutcome::Noop { state } => assert_eq!(state, GoalState::Completed),
            _ => panic!("expected Noop for terminal, got {outcome:?}"),
        }
    }

    #[test]
    fn tick_exhausts_attempts() {
        let (coord, _tmp) = setup();
        let store = coord.store.lock().unwrap();
        let spec = GoalSpec {
            original_intent: "limited goal".into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: GoalBudget {
                max_input_tokens: 100000,
                max_output_tokens: 50000,
                max_cost_usd: None,
                max_attempts: 1,
                deadline_ms: None,
            },
        };
        let g = store
            .create_goal(&PrincipalId("test".into()), "s", "session", &spec)
            .unwrap();
        drop(store);

        coord.tick(g.id, 0).unwrap();
        let outcome1 = coord.tick(g.id, 0).unwrap();
        assert!(matches!(outcome1, GoalTickOutcome::TurnRequested { .. }));

        let outcome2 = coord.tick(g.id, 0).unwrap();
        match outcome2 {
            GoalTickOutcome::BudgetBlocked { .. } => {}
            _ => panic!("expected BudgetBlocked, got {outcome2:?}"),
        }
    }

    #[test]
    fn set_process_link_persists() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        let pid = ProcessId(uuid::Uuid::new_v4());
        let snapshot = coord.set_process_link(g.id, 0, Some(pid)).unwrap();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.process_id, Some(pid));
    }

    #[test]
    fn set_process_link_clear() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        let pid = ProcessId(uuid::Uuid::new_v4());
        coord.set_process_link(g.id, 0, Some(pid)).unwrap();
        let snapshot = coord.set_process_link(g.id, 1, None).unwrap();
        assert_eq!(snapshot.version, 2);
        assert_eq!(snapshot.process_id, None);
    }

    #[test]
    fn set_process_link_stale_version_fails() {
        let (coord, _tmp) = setup();
        let g = create_goal(&coord);
        let pid = ProcessId(uuid::Uuid::new_v4());
        coord.set_process_link(g.id, 0, Some(pid)).unwrap();
        let err = coord.set_process_link(g.id, 0, Some(pid)).unwrap_err();
        match err {
            GoalTransitionError::VersionConflict { .. } => {}
            _ => panic!("expected VersionConflict, got {err:?}"),
        }
    }
}
