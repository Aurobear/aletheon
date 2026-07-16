//! Daemon turn orchestration over narrow lifecycle and pipeline resources.

use crate::service::turn_coordinator::TurnCoordinator;
use crate::service::TurnPipeline;
use aletheon_kernel::KernelRuntime;
use fabric::{OperationId, ProcessId, ProcessSignal};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

pub struct DaemonTurnResources {
    pub kernel: Arc<KernelRuntime>,
    pub notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub default_session_id: Arc<Mutex<String>>,
    pub main_agent_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub pipeline: Arc<TurnPipeline>,
    pub coordinator: Arc<TurnCoordinator>,
    pub session_service: Arc<crate::service::session_service::SessionService>,
}

pub struct DaemonTurnOrchestrator {
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) main_agent_process_id: Arc<Mutex<Option<ProcessId>>>,
    pub(crate) turn_token: Arc<Mutex<Option<CancellationToken>>>,
    pub(crate) pipeline: Arc<TurnPipeline>,
    pub(crate) coordinator: Arc<TurnCoordinator>,
    pub(crate) session_service: Arc<crate::service::session_service::SessionService>,
}

impl DaemonTurnOrchestrator {
    pub fn new(resources: DaemonTurnResources) -> Self {
        Self {
            kernel: resources.kernel,
            notify_tx: resources.notify,
            main_agent_process_id: resources.main_agent_process_id,
            turn_token: resources.turn_token,
            pipeline: resources.pipeline,
            coordinator: resources.coordinator,
            session_service: resources.session_service,
        }
    }

    pub fn notify_tx(&self) -> &Arc<Mutex<Option<mpsc::Sender<String>>>> {
        &self.notify_tx
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

    /// Cancel an in-flight turn operation.
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
}
