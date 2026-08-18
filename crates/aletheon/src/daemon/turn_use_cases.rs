//! Daemon adapter implementing request-facing Turn use cases.

use crate::daemon::DaemonTurnOrchestrator;
use crate::host::request_use_cases::{CognitiveRuntimePort, TurnUseCases};
use application::turn_control::InterruptReason;
use async_trait::async_trait;
use contracts::{
    OperationId, OperationResult, PrincipalContext, PrincipalId, ProcessId, SessionId, ThreadId,
};
use runtime::session_service::{InterruptOutcome, ResumeResult, SessionService};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub struct ProductionTurnUseCases {
    orchestrator: Arc<DaemonTurnOrchestrator>,
    runtime_port: Arc<dyn CognitiveRuntimePort>,
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    sessions: Arc<SessionService>,
}

impl ProductionTurnUseCases {
    pub fn new(
        orchestrator: Arc<DaemonTurnOrchestrator>,
        runtime_port: Arc<dyn CognitiveRuntimePort>,
        cancel_token: Arc<Mutex<Option<CancellationToken>>>,
        sessions: Arc<SessionService>,
    ) -> Self {
        Self {
            orchestrator,
            runtime_port,
            cancel_token,
            sessions,
        }
    }
}

#[async_trait]
impl TurnUseCases for ProductionTurnUseCases {
    async fn watchdog_snapshot(&self) -> application::turn::coordinator::TurnWatchdogSnapshot {
        self.orchestrator
            .coordinator
            .watchdog_snapshot(15 * 60 * 1000)
            .await
    }
    async fn execute(
        &self,
        id: serde_json::Value,
        message: String,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> serde_json::Value {
        self.orchestrator
            .execute_turn_targeted(
                id,
                &message,
                context,
                requirements,
                task_kind,
                execution_target,
                notify,
            )
            .await
    }
    async fn submit(
        &self,
        id: serde_json::Value,
        message: String,
        context: PrincipalContext,
        requirements: Vec<::contracts::TurnRequirement>,
        task_kind: Option<::contracts::TaskKind>,
        execution_target: ::contracts::ExecutionTargetSelection,
        notify: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> anyhow::Result<runtime::TurnId> {
        self.orchestrator
            .submit_turn_targeted(
                id,
                message,
                context,
                requirements,
                task_kind,
                execution_target,
                notify,
            )
            .await
    }
    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        self.orchestrator.wait_turn(id).await
    }
    async fn cancel(&self, id: OperationId) -> anyhow::Result<()> {
        self.orchestrator.cancel_turn(id).await
    }
    async fn cancel_by_key(
        &self,
        principal_id: PrincipalId,
        thread_id: String,
        turn_id: ::contracts::TurnId,
        operation_id: OperationId,
    ) -> anyhow::Result<()> {
        let tid = ThreadId(thread_id);
        self.orchestrator
            .cancel_turn_by_key(&principal_id, &tid, turn_id, operation_id)
            .await
    }
    async fn verify_active(
        &self,
        principal_id: PrincipalId,
        thread_id: String,
        turn_id: ::contracts::TurnId,
        operation_id: OperationId,
    ) -> anyhow::Result<()> {
        self.orchestrator
            .coordinator
            .verify_active_turn(&principal_id, &ThreadId(thread_id), turn_id, operation_id)
            .await
    }
    async fn exit(&self, id: ProcessId) -> anyhow::Result<()> {
        self.orchestrator.exit_process(id).await
    }
    async fn rewind_workspace(
        &self,
        principal_id: PrincipalId,
        session_id: String,
        prompt_index: u64,
        workspace: application::workspace_checkpoint::WorkspaceIdentity,
    ) -> application::workspace_checkpoint::RestoreOutcome {
        self.orchestrator
            .rewind_workspace(&principal_id, &session_id, prompt_index, &workspace)
            .await
    }
    async fn cancel_current(&self) -> usize {
        self.runtime_port
            .request_interrupt(InterruptReason::UserCancelled)
            .await;
        if let Some(token) = self.cancel_token.lock().await.take() {
            token.cancel();
        }
        self.orchestrator.cancel_all_turns().await
    }
    async fn cancel_current_for_principal(&self, principal_id: PrincipalId) -> usize {
        self.runtime_port
            .request_interrupt(InterruptReason::UserCancelled)
            .await;
        if let Some(token) = self.cancel_token.lock().await.take() {
            token.cancel();
        }
        self.orchestrator
            .cancel_turns_for_principal(&principal_id)
            .await
    }
    async fn cancel_current_with_thread(&self, thread_id: String) {
        self.runtime_port
            .request_interrupt(InterruptReason::Timeout)
            .await;
        if let Some(token) = self.cancel_token.lock().await.take() {
            token.cancel();
        }
        let _ = thread_id;
    }
    async fn cancel_active_for_connection(
        &self,
        connection_id: ::contracts::ConnectionId,
    ) -> usize {
        self.orchestrator
            .cancel_turns_for_connection(&connection_id)
            .await
    }
    async fn session_resume(&self, id: SessionId) -> anyhow::Result<ResumeResult> {
        self.sessions.resume(&id).await
    }
    async fn session_fork(
        &self,
        id: SessionId,
        through: u64,
    ) -> anyhow::Result<::contracts::SessionRecord> {
        self.sessions.fork(&id, through).await
    }
    async fn session_sequence_through_turn(
        &self,
        id: SessionId,
        turn_id: ::contracts::TurnId,
    ) -> anyhow::Result<u64> {
        self.sessions.sequence_through_turn(&id, turn_id).await
    }
    async fn session_interrupt(&self, id: SessionId) -> anyhow::Result<InterruptOutcome> {
        self.sessions.interrupt(&id).await
    }
    async fn session_replay(
        &self,
        id: SessionId,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<::contracts::Message>> {
        self.sessions.replay(&id, after).await
    }

    async fn set_notify(&self, sender: tokio::sync::mpsc::Sender<String>) {
        self.orchestrator.set_notify_sender(sender).await;
    }
}
