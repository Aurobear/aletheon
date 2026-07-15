//! DaemonTurnOrchestrator — struct definition and construction.
//!
//! The orchestrator bundles kernel primitives and subsystem handles for
//! executing daemon chat turns. Methods are split across sibling modules
//! in this directory via additional `impl DaemonTurnOrchestrator { … }` blocks.

use crate::core::core_systems::CoreSystems;
use crate::core::session_gateway::SessionGateway;
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::r#impl::daemon::session_manager::SessionManager;
use crate::r#impl::session::canonical_store::{default_session_db_path, CanonicalSessionStore};
use crate::service::turn_coordinator::TurnCoordinator;
use crate::service::TurnPipeline;
use aletheon_kernel::operation::OperationScope;
use aletheon_kernel::KernelRuntime;
use fabric::{
    AdmissionController, AgoraOps, Clock, LlmProvider, OperationId, ProcessId, ProcessSignal,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

/// Bundled state for executing a daemon chat turn through the kernel pipeline.
///
/// Created once per daemon instance (or per `RequestHandler`) and reused across
/// turns. The process table tracks the main agent across its entire lifecycle;
/// per-turn operations are created in `execute_turn()`.
#[allow(dead_code)]
pub struct DaemonTurnOrchestrator {
    // --- Kernel primitives ---
    pub(crate) kernel: Arc<KernelRuntime>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) agora: Option<Arc<dyn AgoraOps>>,

    // --- Subsystem handles (mirrors RequestHandler fields) ---
    pub(crate) subsystems: Arc<CoreSystems>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) llm: Arc<dyn LlmProvider>,
    pub(crate) model_router: Arc<ModelRouter>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) active_connections: Arc<AtomicUsize>,
    pub(crate) started_at: fabric::MonoTime,
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
    /// Per-turn operation scope for structured task cancellation (PR-3).
    ///
    /// Wraps the react task's CancellationToken so `cancel_turn()` can signal
    /// cooperative cancellation. `execute_turn()` drains the scope at turn end
    /// to guarantee no orphan tasks.
    pub(crate) current_scope: Mutex<Option<OperationScope>>,

    // --- Shared turn pipeline ---
    pub(crate) pipeline: Arc<TurnPipeline>,
    pub(crate) coordinator: Arc<TurnCoordinator>,
    pub(crate) session_service: Arc<crate::service::session_service::SessionService>,
}

impl DaemonTurnOrchestrator {
    /// Create the orchestrator, wiring all kernel primitives.
    pub fn new(
        subsystems: Arc<CoreSystems>,
        sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
        session_gateway: Arc<SessionGateway>,
        llm: Arc<dyn LlmProvider>,
        model_router: Arc<ModelRouter>,
        notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
        active_connections: Arc<AtomicUsize>,
        started_at: fabric::MonoTime,
        daemon_cancel_token: Option<CancellationToken>,
        context_assembler: Arc<crate::service::context_assembler::ContextAssembler>,
    ) -> Self {
        // Clone the one opaque Kernel runtime; cognitive domains stay separate.
        let clock = subsystems.kernel.clock();
        let kernel = subsystems.kernel.clone();
        let admission = kernel.admission();
        let agora = Some(subsystems.domains.agora());

        let session_db = default_session_db_path();
        if let Some(parent) = session_db.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let canonical_store = CanonicalSessionStore::open(&session_db).unwrap_or_else(|error| {
            tracing::warn!(%error, path = %session_db.display(), "canonical session store unavailable; using process-local fallback");
            CanonicalSessionStore::open(":memory:").expect("in-memory canonical session store")
        });
        let coordinator = Arc::new(TurnCoordinator::new(
            kernel.clone(),
            Arc::new(canonical_store),
        ));
        let session_service = Arc::new(crate::service::session_service::SessionService::new(
            coordinator.store(),
            coordinator.active_index(),
        ));
        let pipeline = Arc::new(TurnPipeline::new(
            subsystems.clone(),
            sessions.clone(),
            session_gateway.clone(),
            llm.clone(),
            model_router.clone(),
            notify_tx.clone(),
            daemon_cancel_token.clone(),
            context_assembler,
            session_service.clone(),
        ));

        Self {
            kernel,
            clock,
            admission,
            agora,
            subsystems,
            sessions,
            session_gateway,
            llm,
            model_router,
            notify_tx,
            active_connections,
            started_at,
            daemon_cancel_token,
            current_scope: Mutex::new(None),
            pipeline,
            coordinator,
            session_service,
        }
    }

    /// Access the shared notify_tx for external updates (e.g. set_notify_channel).
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
