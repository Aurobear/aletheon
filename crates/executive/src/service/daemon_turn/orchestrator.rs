//! DaemonTurnOrchestrator — struct definition and construction.
//!
//! The orchestrator bundles kernel primitives and subsystem handles for
//! executing daemon chat turns. Methods are split across sibling modules
//! in this directory via additional `impl DaemonTurnOrchestrator { … }` blocks.

use crate::core::core_systems::CoreSystems;
use crate::core::session_gateway::SessionGateway;
use crate::kernel::operation::OperationTable;
use crate::kernel::process::ProcessTable;
use crate::kernel::supervision::SupervisorTree;
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::r#impl::daemon::session_manager::SessionManager;
use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::{AdmissionController, Clock, LlmProvider, ProcessId};
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;
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
    pub(crate) process_table: Arc<ProcessTable>,
    pub(crate) operation_table: Arc<OperationTable>,
    pub(crate) supervisor: Arc<Mutex<SupervisorTree>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) mailbox_service: Arc<InProcessMailboxService>,
    pub(crate) main_agent_process_id: Mutex<Option<ProcessId>>,

    // --- Subsystem handles (mirrors RequestHandler fields) ---
    pub(crate) subsystems: Arc<CoreSystems>,
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    pub(crate) session_gateway: Arc<SessionGateway>,
    pub(crate) llm: Arc<dyn LlmProvider>,
    pub(crate) model_router: Arc<ModelRouter>,
    pub(crate) notify_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub(crate) active_connections: Arc<AtomicUsize>,
    pub(crate) started_at: Instant,
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
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
        started_at: Instant,
        daemon_cancel_token: Option<CancellationToken>,
    ) -> Self {
        // Clone kernel primitives from the canonical ServicePorts instead of
        // constructing independent instances (PR-1: fix double-instance).
        let clock = subsystems.ports.clock.clone();
        let process_table = subsystems.ports.process_table.clone();
        let operation_table = subsystems.ports.operation_table.clone();
        let supervisor = subsystems.ports.supervisor.clone();
        let admission = subsystems.ports.admission.clone();
        let mailbox_service = subsystems.ports.mailbox_service.clone();

        Self {
            process_table,
            operation_table,
            supervisor,
            clock,
            admission,
            mailbox_service,
            main_agent_process_id: Mutex::new(None),
            subsystems,
            sessions,
            session_gateway,
            llm,
            model_router,
            notify_tx,
            active_connections,
            started_at,
            daemon_cancel_token,
        }
    }

    /// Access the shared notify_tx for external updates (e.g. set_notify_channel).
    pub fn notify_tx(&self) -> &Arc<Mutex<Option<mpsc::Sender<String>>>> {
        &self.notify_tx
    }
}
