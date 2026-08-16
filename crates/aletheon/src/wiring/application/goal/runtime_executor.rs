//! Goal attempt adapter for the Runtime-owned DelegateBackend seam.
//!
//! This module translates Goal tasks into a typed Runtime command. Runtime mints
//! the run/generation, fences the terminal, and owns cancellation admission;
//! this adapter keeps only the backend's in-flight execution evidence.

use super::attempt_coordinator::AttemptExecutor;
use ::contracts::{AttemptUsage, FailureClass, RuntimeFailure, RuntimeId, RuntimeResult};
use async_trait::async_trait;
use runtime::{
    DelegateBackend, DelegateBackendId, DelegateCommand, DelegateReceipt, DelegateResult,
    DelegateSpawnRequest, RuntimeAgentSupervisor, RuntimeError, TurnTerminal,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct AttemptRun {
    cancel: CancellationToken,
    result: Arc<Mutex<Option<Result<RuntimeResult, RuntimeFailure>>>>,
    done: Arc<Notify>,
}

/// Compatibility execution binding for one configured Goal runtime. It does
/// not allocate identities or publish terminal events; both remain Runtime
/// responsibilities.
pub struct GoalAttemptBackend {
    legacy_backend: Arc<dyn runtime::DelegateTaskBackend>,
    runs: Mutex<HashMap<runtime::AgentRunId, AttemptRun>>,
}

impl GoalAttemptBackend {
    pub fn new(runtime: Arc<dyn runtime::DelegateTaskBackend>) -> Self {
        Self {
            legacy_backend: runtime,
            runs: Mutex::new(HashMap::new()),
        }
    }

    fn run(&self, id: &runtime::AgentRunId) -> Result<AttemptRun, RuntimeError> {
        self.runs
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .get(id)
            .map(|run| AttemptRun {
                cancel: run.cancel.clone(),
                result: run.result.clone(),
                done: run.done.clone(),
            })
            .ok_or(RuntimeError::AgentRunNotFound)
    }
}

#[async_trait]
impl DelegateBackend for GoalAttemptBackend {
    async fn spawn(
        &self,
        request: &DelegateSpawnRequest,
        identity: &DelegateReceipt,
    ) -> Result<DelegateReceipt, RuntimeError> {
        let DelegateCommand::GoalAttempt { task } = request
            .command
            .as_ref()
            .ok_or(RuntimeError::UnsupportedRequest)?;
        if task.trim().is_empty() || task.len() > ::contracts::agent_control::MAX_AGENT_TASK_BYTES {
            return Err(RuntimeError::UnsupportedRequest);
        }
        if request.host_request.is_some() {
            return Err(RuntimeError::UnsupportedRequest);
        }
        let cancel = CancellationToken::new();
        let result = Arc::new(Mutex::new(None));
        let done = Arc::new(Notify::new());
        let legacy_backend = self.legacy_backend.clone();
        let task = task.clone();
        let task_result = result.clone();
        let task_done = done.clone();
        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            let outcome = legacy_backend.run_attempt(&task, task_cancel).await;
            if let Ok(mut slot) = task_result.lock() {
                *slot = Some(outcome);
            }
            task_done.notify_waiters();
        });
        self.runs
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .insert(
                identity.agent_run.clone(),
                AttemptRun {
                    cancel,
                    result,
                    done,
                },
            );
        Ok(identity.clone())
    }

    async fn cancel(&self, agent_run: &runtime::AgentRunId) -> Result<(), RuntimeError> {
        self.run(agent_run)?.cancel.cancel();
        Ok(())
    }

    async fn wait(&self, agent_run: &runtime::AgentRunId) -> Result<TurnTerminal, RuntimeError> {
        let run = self.run(agent_run)?;
        loop {
            let notified = run.done.notified();
            if let Some(outcome) = run
                .result
                .lock()
                .map_err(|_| RuntimeError::Internal)?
                .clone()
            {
                return Ok(match outcome {
                    Ok(_) => TurnTerminal::Completed,
                    Err(error) => TurnTerminal::Failed {
                        message: error.message,
                    },
                });
            }
            notified.await;
        }
    }

    async fn result(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<DelegateResult, RuntimeError> {
        let run = self.run(agent_run)?;
        let outcome = run
            .result
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .clone()
            .ok_or(RuntimeError::NotTerminal)?;
        Ok(match outcome {
            Ok(result) => DelegateResult::Succeeded(result),
            Err(error) => DelegateResult::Failed(error),
        })
    }
}

/// Goal-facing executor. It selects only from Runtime-published backend IDs;
/// no Aletheon-owned provider registry is visible at this boundary.
pub struct RuntimeGoalAttemptExecutor {
    supervisor: Arc<RuntimeAgentSupervisor>,
    backend_ids: HashMap<RuntimeId, DelegateBackendId>,
    parent_session: runtime::SessionId,
}

impl RuntimeGoalAttemptExecutor {
    pub fn new(
        supervisor: Arc<RuntimeAgentSupervisor>,
        backend_ids: HashMap<RuntimeId, DelegateBackendId>,
        parent_session: runtime::SessionId,
    ) -> Self {
        Self {
            supervisor,
            backend_ids,
            parent_session,
        }
    }
}

#[async_trait]
impl AttemptExecutor for RuntimeGoalAttemptExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        self.backend_ids
            .get(runtime_id)
            .is_some_and(|backend| self.supervisor.registry().resolve(backend).is_some())
    }

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        let backend = self
            .backend_ids
            .get(runtime_id)
            .ok_or_else(|| RuntimeFailure {
                class: FailureClass::MissingDependency,
                message: format!("Goal runtime is not registered: {}", runtime_id.0),
                retryable: false,
                usage: AttemptUsage::default(),
                evidence: vec![],
            })?;
        let receipt = self
            .supervisor
            .spawn_request(runtime::DelegateSpawnRequest {
                parent_session: self.parent_session.clone(),
                // Goal attempts are background work and have no admitted
                // parent turn. Runtime owns TurnId allocation; do not create
                // a synthetic client-side identity here.
                parent_turn: None,
                backend: backend.clone(),
                profile: None,
                host_request: None,
                command: Some(DelegateCommand::GoalAttempt {
                    task: task.to_owned(),
                }),
            })
            .await
            .map_err(runtime_failure)?;
        let terminal = tokio::select! {
            terminal = self.supervisor.wait_with_generation(&receipt.agent_run, &receipt.generation) => terminal.map_err(runtime_failure)?,
            _ = cancel.cancelled() => {
                let _ = self.supervisor.cancel_with_generation(&receipt.agent_run, &receipt.generation).await;
                return Err(RuntimeFailure {
                    class: FailureClass::Cancelled,
                    message: "Goal runtime attempt cancelled".into(),
                    retryable: false,
                    usage: AttemptUsage::default(),
                    evidence: vec![],
                });
            }
        };
        match terminal {
            TurnTerminal::Completed | TurnTerminal::Failed { .. } => match self
                .supervisor
                .result_with_generation(&receipt.agent_run, &receipt.generation)
                .await
                .map_err(runtime_failure)?
            {
                DelegateResult::Succeeded(result) => Ok(result),
                DelegateResult::Failed(error) => Err(error),
            },
            TurnTerminal::Interrupted => Err(RuntimeFailure {
                class: FailureClass::Cancelled,
                message: "Goal runtime attempt interrupted".into(),
                retryable: false,
                usage: AttemptUsage::default(),
                evidence: vec![],
            }),
        }
    }
}

fn runtime_failure(error: RuntimeError) -> RuntimeFailure {
    RuntimeFailure {
        class: FailureClass::MissingDependency,
        message: error.to_string(),
        retryable: false,
        usage: AttemptUsage::default(),
        evidence: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FixedRuntime;

    #[async_trait]
    impl runtime::DelegateTaskBackend for FixedRuntime {
        async fn run_attempt(
            &self,
            task: &str,
            _cancel: CancellationToken,
        ) -> Result<RuntimeResult, RuntimeFailure> {
            Ok(RuntimeResult {
                output: format!("done:{task}"),
                ..RuntimeResult::default()
            })
        }
    }

    #[tokio::test]
    async fn runtime_goal_executor_uses_typed_command_and_runtime_terminal() {
        let supervisor = Arc::new(RuntimeAgentSupervisor::new(
            runtime::DelegateBackendRegistry::new(),
        ));
        let backend_id = DelegateBackendId("goal-attempt:test".into());
        supervisor
            .register_backend(
                backend_id.clone(),
                Arc::new(GoalAttemptBackend::new(Arc::new(FixedRuntime))),
            )
            .unwrap();
        let executor = RuntimeGoalAttemptExecutor::new(
            supervisor,
            HashMap::from([(RuntimeId("test".into()), backend_id)]),
            runtime::SessionId("session".into()),
        );
        let result = executor
            .run_once(&RuntimeId("test".into()), "hello", CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.output, "done:hello");
    }
}
