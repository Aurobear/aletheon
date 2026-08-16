//! Daemon turn orchestration over narrow lifecycle and pipeline resources.

use crate::wiring::application::turn_coordinator::TurnCoordinator;
use crate::wiring::application::turn_runtime_ports::ActiveAgentProfilePort;
use crate::wiring::application::TurnPipeline;
use ::contracts::{OperationId, PrincipalId, ProcessId, ProcessSignal, ThreadId, TurnId};
use crate::config::GrokHardeningConfig;
use kernel::KernelRuntime;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
pub type TestTurnRunner = Arc<
    dyn Fn(
            ::contracts::TurnRequest,
            CancellationToken,
        ) -> futures::future::BoxFuture<
            'static,
            anyhow::Result<crate::wiring::application::turn_coordinator::TurnExecution>,
        > + Send
        + Sync,
>;

pub(crate) struct DaemonTurnResources {
    pub kernel: Arc<KernelRuntime>,
    pub notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub main_agent_process_ids: Arc<Mutex<HashMap<String, ProcessId>>>,
    pub approval_owner_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub pipeline: Arc<TurnPipeline>,
    pub coordinator: Arc<TurnCoordinator>,
    pub session_service: Arc<crate::wiring::application::session_service::SessionService>,
    pub grok_hardening: GrokHardeningConfig,
    pub active_profile: Arc<dyn ActiveAgentProfilePort>,
}

pub struct DaemonTurnOrchestrator {
    pub kernel: Arc<KernelRuntime>,
    pub notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub main_agent_process_ids: Arc<Mutex<HashMap<String, ProcessId>>>,
    pub approval_owner_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub pipeline: Option<Arc<TurnPipeline>>,
    pub turn_engine: Option<Arc<dyn crate::wiring::application::turn_engine::TurnEngine>>,
    pub coordinator: Arc<TurnCoordinator>,
    pub session_service: Arc<crate::wiring::application::session_service::SessionService>,
    #[allow(dead_code)]
    pub grok_hardening: GrokHardeningConfig,
    pub active_profile: Arc<dyn ActiveAgentProfilePort>,
    #[cfg(test)]
    pub test_runner: Option<TestTurnRunner>,
}

impl DaemonTurnOrchestrator {
    /// Public composition entry for the binary-owned wiring crate. The
    /// resource bag itself remains crate-private so callers cannot retain or
    /// construct the daemon component graph outside this boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        kernel: Arc<KernelRuntime>,
        notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
        main_agent_process_ids: Arc<Mutex<HashMap<String, ProcessId>>>,
        approval_owner_process_id: Arc<Mutex<Option<ProcessId>>>,
        turn_token: Arc<Mutex<Option<CancellationToken>>>,
        pipeline: Arc<TurnPipeline>,
        coordinator: Arc<TurnCoordinator>,
        session_service: Arc<crate::wiring::application::session_service::SessionService>,
        grok_hardening: GrokHardeningConfig,
        active_profile: Arc<dyn ActiveAgentProfilePort>,
    ) -> Self {
        Self::new(DaemonTurnResources {
            kernel,
            notify,
            main_agent_process_ids,
            approval_owner_process_id,
            turn_token,
            pipeline,
            coordinator,
            session_service,
            grok_hardening,
            active_profile,
        })
    }

    pub(crate) fn new(resources: DaemonTurnResources) -> Self {
        let turn_engine = Arc::new(
            crate::wiring::application::daemon_turn_engine::DaemonTurnEngine::new(
                resources.pipeline.clone(),
            ),
        );
        Self {
            kernel: resources.kernel,
            notify_tx: resources.notify,
            main_agent_process_ids: resources.main_agent_process_ids,
            approval_owner_process_id: resources.approval_owner_process_id,
            turn_token: resources.turn_token,
            pipeline: Some(resources.pipeline),
            turn_engine: Some(turn_engine),
            coordinator: resources.coordinator,
            session_service: resources.session_service,
            grok_hardening: resources.grok_hardening,
            active_profile: resources.active_profile,
            #[cfg(test)]
            test_runner: None,
        }
    }

    /// Install the current turn-notification sender without exposing the
    /// underlying lock. Waiting for the short assignment prevents a reconnect
    /// from silently retaining the previous connection's sender.
    pub async fn set_notify_sender(&self, sender: mpsc::Sender<String>) {
        *self.notify_tx.lock().await = Some(sender);
    }

    // ── Public kernel API — wait / cancel / exit (PR-3) ──────────────────

    /// Wait for an operation to reach a terminal state.
    ///
    /// Delegates to the kernel runtime, which blocks until the operation
    /// transitions to Succeeded, Failed, or Cancelled.
    pub async fn wait_turn(
        &self,
        operation_id: OperationId,
    ) -> anyhow::Result<::contracts::OperationResult> {
        self.kernel.wait_operation(operation_id).await
    }

    /// Cancel an in-flight turn operation (legacy: operation_id only).
    ///
    /// 1. Cancels the per-turn `OperationScope`'s `CancellationToken` so the
    ///    react task can cooperatively exit before its next tool call.
    /// 2. Propagates cancellation through the operation tree in the kernel
    ///    operation tree (parent → children).
    pub async fn cancel_turn(&self, operation_id: OperationId) -> anyhow::Result<()> {
        if self.coordinator.cancel_operation(operation_id).await {
            Ok(())
        } else {
            anyhow::bail!("turn operation is not active")
        }
    }

    pub async fn cancel_turns_for_principal(&self, principal_id: &PrincipalId) -> usize {
        self.coordinator
            .cancel_active_for_principal(principal_id)
            .await
    }

    /// Cancel exactly the active turns admitted by one transport connection.
    pub async fn cancel_turns_for_connection(
        &self,
        connection_id: &::contracts::ConnectionId,
    ) -> usize {
        self.coordinator
            .cancel_active_for_connection(connection_id)
            .await
    }

    pub async fn cancel_all_turns(&self) -> usize {
        self.coordinator.cancel_all_active().await
    }

    /// Cancel an in-flight turn with identity-aware lookup (G3 prompt_queue).
    ///
    /// When `grok_hardening.prompt_queue` is enabled, this validates the
    /// cancel authority via `evaluate_cancel` before cancelling.
    pub async fn cancel_turn_by_key(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        turn_id: TurnId,
        operation_id: OperationId,
    ) -> anyhow::Result<()> {
        self.coordinator
            .cancel_operation_by_key(principal_id, thread_id, turn_id, operation_id)
            .await
    }

    /// Signal a process to exit (Terminate).
    ///
    /// Delegates to the kernel runtime. The process transitions through
    /// Stopping → Exited/Failed, and any in-flight operations are cancelled via
    /// the operation tree's parent-cancel propagation.
    pub async fn exit_process(&self, process_id: ProcessId) -> anyhow::Result<()> {
        self.kernel
            .signal_process(process_id, ProcessSignal::Terminate)
            .await
    }

    pub async fn rewind_workspace(
        &self,
        principal_id: &PrincipalId,
        session_id: &str,
        prompt_index: u64,
        workspace: &application::workspace_checkpoint::WorkspaceIdentity,
    ) -> application::workspace_checkpoint::RestoreOutcome {
        self.pipeline
            .as_ref()
            .expect("production daemon orchestrator has a turn pipeline")
            .workspace_checkpoint
            .rewind_to(
                principal_id,
                session_id,
                prompt_index,
                workspace,
                self.pipeline
                    .as_ref()
                    .expect("production daemon orchestrator has a turn pipeline")
                    .clock
                    .mono_now()
                    .0,
            )
            .await
    }
}
