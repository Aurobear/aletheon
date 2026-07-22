//! Daemon turn orchestration over narrow lifecycle and pipeline resources.

use crate::core::config::GrokHardeningConfig;
use crate::service::turn_coordinator::TurnCoordinator;
use crate::service::turn_runtime_ports::ActiveAgentProfilePort;
use crate::service::TurnPipeline;
use fabric::{OperationId, PrincipalId, ProcessId, ProcessSignal, ThreadId, TurnId};
use kernel::KernelRuntime;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
pub(crate) type TestTurnRunner = Arc<
    dyn Fn(
            fabric::TurnRequest,
            CancellationToken,
        ) -> futures::future::BoxFuture<
            'static,
            anyhow::Result<crate::service::turn_coordinator::TurnExecution>,
        > + Send
        + Sync,
>;

pub(crate) struct DaemonTurnResources {
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) main_agent_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub(crate) turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub(crate) pipeline: Arc<TurnPipeline>,
    pub(crate) coordinator: Arc<TurnCoordinator>,
    pub(crate) session_service: Arc<crate::service::session_service::SessionService>,
    pub(crate) grok_hardening: GrokHardeningConfig,
    pub(crate) active_profile: Arc<dyn ActiveAgentProfilePort>,
}

pub struct DaemonTurnOrchestrator {
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) main_agent_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub(crate) turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub(crate) pipeline: Option<Arc<TurnPipeline>>,
    pub(crate) turn_engine: Option<Arc<dyn crate::service::turn_engine::TurnEngine>>,
    pub(crate) coordinator: Arc<TurnCoordinator>,
    pub(crate) session_service: Arc<crate::service::session_service::SessionService>,
    #[allow(dead_code)]
    pub(crate) grok_hardening: GrokHardeningConfig,
    pub(crate) active_profile: Arc<dyn ActiveAgentProfilePort>,
    #[cfg(test)]
    pub(crate) test_runner: Option<TestTurnRunner>,
}

impl DaemonTurnOrchestrator {
    pub(crate) fn new(resources: DaemonTurnResources) -> Self {
        let turn_engine = Arc::new(crate::service::daemon_turn_engine::DaemonTurnEngine::new(
            resources.pipeline.clone(),
        ));
        Self {
            kernel: resources.kernel,
            notify_tx: resources.notify,
            main_agent_process_id: resources.main_agent_process_id,
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
    ) -> anyhow::Result<fabric::OperationResult> {
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
        workspace: &fabric::types::workspace_checkpoint::WorkspaceIdentity,
    ) -> fabric::types::workspace_checkpoint::RestoreOutcome {
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
