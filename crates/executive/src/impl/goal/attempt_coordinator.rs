//! One-shot durable Goal attempt coordination.
//!
//! A call performs exactly one runtime invocation. Any retry or escalation is
//! represented as durable Goal state for a later scheduler tick.

use super::budget::{GoalBudgetError, GoalBudgetRequest};
use super::transition::GoalTransitionError;
use super::{GoalAttempt, GoalFrame, ObjectiveStore, RetryDecision, RetryPolicy};
use crate::core::runtime_registry::RuntimeRegistry;
use async_trait::async_trait;
use fabric::{
    AttemptUsage, Clock, CognitiveRole, FailureClass, GoalBudgetUsage, GoalId, GoalSnapshot,
    GoalState, GoalWaitReason, RuntimeFailure, RuntimeId, RuntimeResult,
};
use std::fmt;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Immutable input for exactly one runtime invocation.
#[derive(Debug, Clone)]
pub struct AttemptRequest {
    pub goal_id: GoalId,
    pub expected_version: u64,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub escalation_runtime_id: Option<RuntimeId>,
    pub role: CognitiveRole,
    pub task: String,
    /// Admission estimate; actual metering replaces it during settlement.
    pub estimated_usage: AttemptUsage,
}

/// Result of one coordinator call. It never contains a second invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum AttemptCoordinationOutcome {
    Succeeded {
        attempt: GoalAttempt,
        goal: GoalSnapshot,
    },
    Failed {
        attempt: GoalAttempt,
        decision: RetryDecision,
        goal: GoalSnapshot,
    },
}

#[derive(Debug)]
pub enum AttemptCoordinatorError {
    GoalNotFound(GoalId),
    GoalNotRunning(GoalState),
    VersionConflict { expected: u64, actual: u64 },
    RuntimeUnavailable(RuntimeId),
    Budget(GoalBudgetError),
    Persistence(String),
    Transition(GoalTransitionError),
}

impl fmt::Display for AttemptCoordinatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GoalNotFound(id) => write!(f, "goal {id} not found"),
            Self::GoalNotRunning(state) => write!(f, "goal is not running: {state}"),
            Self::VersionConflict { expected, actual } => {
                write!(f, "version conflict: expected {expected}, actual {actual}")
            }
            Self::RuntimeUnavailable(id) => write!(f, "runtime unavailable: {}", id.0),
            Self::Budget(error) => write!(f, "budget error: {error}"),
            Self::Persistence(error) => write!(f, "attempt persistence error: {error}"),
            Self::Transition(error) => write!(f, "goal transition error: {error}"),
        }
    }
}

impl std::error::Error for AttemptCoordinatorError {}

/// Boundary used to resolve and invoke a configured runtime once.
#[async_trait]
pub trait AttemptExecutor: Send + Sync {
    /// Must not create a process or invoke a provider.
    fn is_available(&self, runtime_id: &RuntimeId) -> bool;

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure>;
}

/// Production executor backed by the configured RuntimeRegistry.
pub struct RegistryAttemptExecutor {
    registry: Arc<RuntimeRegistry>,
}

impl RegistryAttemptExecutor {
    pub fn new(registry: Arc<RuntimeRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl AttemptExecutor for RegistryAttemptExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        self.registry.contains(runtime_id)
    }

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        let runtime = self
            .registry
            .resolve(runtime_id)
            .map_err(|error| RuntimeFailure {
                class: FailureClass::MissingDependency,
                message: error.to_string(),
                retryable: false,
                usage: AttemptUsage::default(),
                evidence: vec![],
            })?;
        runtime.run_attempt(task, cancel).await
    }
}

pub struct AttemptCoordinator {
    store: Arc<Mutex<ObjectiveStore>>,
    executor: Arc<dyn AttemptExecutor>,
    clock: Arc<dyn Clock>,
    retry_policy: RetryPolicy,
}

impl AttemptCoordinator {
    pub fn new(
        store: Arc<Mutex<ObjectiveStore>>,
        executor: Arc<dyn AttemptExecutor>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            store,
            executor,
            clock,
            retry_policy,
        }
    }

    /// Execute exactly one durable attempt and persist the next Goal state.
    pub async fn execute_one(
        &self,
        request: AttemptRequest,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        // Resolve before budget reservation or attempt creation.
        if !self.executor.is_available(&request.runtime_id) {
            return Err(AttemptCoordinatorError::RuntimeUnavailable(
                request.runtime_id,
            ));
        }

        let reservation_id;
        let running_attempt;
        let rendered_task;
        {
            let store = self.store.lock().unwrap();
            let goal = store
                .get_goal(request.goal_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .ok_or(AttemptCoordinatorError::GoalNotFound(request.goal_id))?;
            if goal.state != GoalState::Running {
                return Err(AttemptCoordinatorError::GoalNotRunning(goal.state));
            }
            if goal.version != request.expected_version {
                return Err(AttemptCoordinatorError::VersionConflict {
                    expected: request.expected_version,
                    actual: goal.version,
                });
            }

            let previous_attempts = store
                .attempts_for_goal(request.goal_id, usize::MAX)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
            let frame = GoalFrame::build(&goal, &previous_attempts, &request.task);
            rendered_task = frame.render();

            let reservation = store
                .reserve_goal_budget(
                    request.goal_id,
                    GoalBudgetRequest {
                        input_tokens: request.estimated_usage.input_tokens,
                        output_tokens: request.estimated_usage.output_tokens,
                        cost_usd: request.estimated_usage.cost_usd.unwrap_or_default(),
                        attempts: 1,
                    },
                    self.clock.wall_now().0,
                )
                .map_err(AttemptCoordinatorError::Budget)?;
            reservation_id = reservation.reservation_id;

            let input = serde_json::json!({
                "task": request.task,
                "goal_version": request.expected_version,
                "goal_frame": frame,
            });
            match store.begin_attempt(
                request.goal_id,
                request.sequence,
                &request.runtime_id,
                request.role,
                &input,
            ) {
                Ok(attempt) => running_attempt = attempt,
                Err(error) => {
                    let _ = store.revoke_goal_budget(&reservation_id);
                    return Err(AttemptCoordinatorError::Persistence(error.to_string()));
                }
            }
        }

        let runtime_outcome = tokio::select! {
            outcome = self.executor.run_once(&request.runtime_id, &rendered_task, cancel.clone()) => outcome,
            _ = cancel.cancelled() => Err(cancelled_failure()),
        };

        let terminal_attempt = {
            let store = self.store.lock().unwrap();
            let persisted = match &runtime_outcome {
                Err(failure) if failure.class == FailureClass::Cancelled => {
                    store.cancel_attempt(running_attempt.id, failure.clone())
                }
                _ => store.finish_attempt(running_attempt.id, runtime_outcome.clone()),
            };
            let attempt = match persisted {
                Ok(attempt) => attempt,
                Err(error) => {
                    let _ = store.revoke_goal_budget(&reservation_id);
                    return Err(AttemptCoordinatorError::Persistence(error.to_string()));
                }
            };
            store
                .settle_goal_budget(&reservation_id, usage_for_budget(&attempt.usage))
                .map_err(AttemptCoordinatorError::Budget)?;
            attempt
        };

        match runtime_outcome {
            Ok(_) => {
                let goal = self.transition_after(
                    request.goal_id,
                    GoalState::Completed,
                    None,
                    serde_json::json!({
                        "action": "attempt_succeeded",
                        "attempt_id": terminal_attempt.id.0,
                    }),
                )?;
                Ok(AttemptCoordinationOutcome::Succeeded {
                    attempt: terminal_attempt,
                    goal,
                })
            }
            Err(failure) => {
                let attempt_count = {
                    let store = self.store.lock().unwrap();
                    store
                        .attempts_for_goal(request.goal_id, usize::MAX)
                        .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                        .into_iter()
                        .filter(|attempt| attempt.role == request.role)
                        .count() as u32
                };
                let decision = self.retry_policy.decide(
                    request.role,
                    attempt_count,
                    &failure,
                    request.escalation_runtime_id.as_ref(),
                );
                let goal = self.persist_decision(
                    request.goal_id,
                    terminal_attempt.id.0.to_string(),
                    &decision,
                )?;
                Ok(AttemptCoordinationOutcome::Failed {
                    attempt: terminal_attempt,
                    decision,
                    goal,
                })
            }
        }
    }

    fn persist_decision(
        &self,
        goal_id: GoalId,
        attempt_id: String,
        decision: &RetryDecision,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError> {
        match decision {
            RetryDecision::RetrySame { after_ms, .. } => {
                let until_ms = self
                    .clock
                    .wall_now()
                    .0
                    .saturating_add((*after_ms).min(i64::MAX as u64) as i64);
                self.transition_after(
                    goal_id,
                    GoalState::Blocked,
                    Some(GoalWaitReason::Backoff { until_ms }),
                    serde_json::json!({
                        "action": "retry_scheduled",
                        "attempt_id": attempt_id,
                        "until_ms": until_ms,
                    }),
                )
            }
            RetryDecision::Escalate { runtime_id, .. } => self.transition_after(
                goal_id,
                GoalState::Blocked,
                Some(GoalWaitReason::ExternalEvent {
                    key: format!("runtime:{}", runtime_id.0),
                }),
                serde_json::json!({
                    "action": "runtime_escalated",
                    "attempt_id": attempt_id,
                    "runtime_id": runtime_id.0,
                }),
            ),
            RetryDecision::AwaitHuman { reason } => self.transition_after(
                goal_id,
                GoalState::AwaitingHuman,
                Some(GoalWaitReason::HumanInput {
                    prompt: reason.clone(),
                }),
                serde_json::json!({"action": "await_human", "attempt_id": attempt_id}),
            ),
            RetryDecision::Fail { reason } => self.transition_after(
                goal_id,
                GoalState::Failed,
                None,
                serde_json::json!({
                    "action": "attempt_failed_terminal",
                    "attempt_id": attempt_id,
                    "reason": reason,
                }),
            ),
            RetryDecision::Cancel => self.transition_after(
                goal_id,
                GoalState::Cancelled,
                None,
                serde_json::json!({"action": "attempt_cancelled", "attempt_id": attempt_id}),
            ),
        }
    }

    fn transition_after(
        &self,
        goal_id: GoalId,
        state: GoalState,
        wait_reason: Option<GoalWaitReason>,
        payload: serde_json::Value,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap();
        let current = store
            .get_goal(goal_id)
            .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
            .ok_or(AttemptCoordinatorError::GoalNotFound(goal_id))?;
        store
            .transition_goal(
                goal_id,
                current.version,
                state,
                wait_reason.as_ref(),
                &payload,
            )
            .map_err(AttemptCoordinatorError::Transition)
    }
}

fn cancelled_failure() -> RuntimeFailure {
    RuntimeFailure {
        class: FailureClass::Cancelled,
        message: "attempt cancelled".into(),
        retryable: false,
        usage: AttemptUsage::default(),
        evidence: vec![],
    }
}

fn usage_for_budget(usage: &AttemptUsage) -> GoalBudgetUsage {
    GoalBudgetUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd.unwrap_or_default(),
        attempts: 1,
    }
}
