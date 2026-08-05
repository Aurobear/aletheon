//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

use crate::application::request_use_cases::{
    ExternalSourceUseCases, HealthUseCases, ReflectionUseCases, SessionLifecycleUseCases,
    TurnUseCases, WorkflowUseCases,
};
use crate::application::{AdminUseCases, ApprovalUseCases, GoalUseCases};
use crate::compatibility::legacy_session_service::LegacySessionUseCases;
use crate::host::daemon::debug_handler::DebugHandler;

pub(crate) struct TransportPorts {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) capabilities_grant: corpus::ExtensionGrant,
    pub(crate) capabilities: Arc<dyn crate::application::governed_capability::CapabilityService>,
    pub(crate) clock: Arc<dyn fabric::Clock>,
}

pub(crate) struct HandlerPorts {
    pub(crate) kernel: Arc<kernel::KernelRuntime>,
    pub(crate) pending_approvals: crate::application::admin_service::PendingApprovals,
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
    pub(crate) approvals: Arc<dyn ApprovalUseCases>,
    pub(crate) admin: Arc<dyn AdminUseCases>,
    pub(crate) sessions: Arc<dyn LegacySessionUseCases>,
    pub(crate) session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
    pub(crate) health: Arc<dyn HealthUseCases>,
    // Retained for host-owned governance/diagnostics; no public RPC route may
    // call this port directly.
    pub(crate) _reflection: Arc<dyn ReflectionUseCases>,
    pub(crate) google: Arc<dyn ExternalSourceUseCases>,
    pub(crate) workflow: Arc<dyn WorkflowUseCases>,
    pub(crate) turn: Arc<dyn TurnUseCases>,
    pub(crate) evaluation: Arc<crate::application::evaluation::EvaluationService>,
    pub(crate) workspace_checkpoint:
        Arc<crate::application::workspace_checkpoint::WorkspaceCheckpointService>,
    pub(crate) transaction_review: Arc<crate::application::settlement::TransactionReviewService>,
    pub(crate) session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
    pub(crate) conscious_workspaces:
        Arc<crate::application::conscious_workspace::ConsciousWorkspaceRegistry>,
    pub(crate) debug: Arc<DebugHandler>,
    pub(crate) session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
    pub(crate) recall_service: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) memory_gateway: Arc<crate::application::memory_gateway::MemoryGatewayService>,
    pub(crate) memory_maintenance:
        Arc<crate::application::memory_maintenance::MemoryMaintenanceController>,
    pub(crate) memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
    pub(crate) inference: Arc<dyn crate::application::inference_port::InferencePort>,
    pub(crate) review: Option<Arc<crate::application::governed_review::GovernedReviewService>>,
    pub(crate) extensions: Arc<crate::application::extension_coordinator::ExtensionCoordinator>,
    pub(crate) transport: Arc<TransportPorts>,
}

impl HandlerPorts {
    pub(crate) fn memory_health_snapshot(&self) -> mnemosyne::CompositeMemoryHealth {
        self.memory_health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn new(
        kernel: Arc<kernel::KernelRuntime>,
        pending_approvals: crate::application::admin_service::PendingApprovals,
        facts: Arc<dyn FactUseCases>,
        goals: Arc<dyn GoalUseCases>,
        approvals: Arc<dyn ApprovalUseCases>,
        admin: Arc<dyn AdminUseCases>,
        sessions: Arc<dyn LegacySessionUseCases>,
        session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
        health: Arc<dyn HealthUseCases>,
        reflection: Arc<dyn ReflectionUseCases>,
        google: Arc<dyn ExternalSourceUseCases>,
        workflow: Arc<dyn WorkflowUseCases>,
        turn: Arc<dyn TurnUseCases>,
        evaluation: Arc<crate::application::evaluation::EvaluationService>,
        workspace_checkpoint: Arc<
            crate::application::workspace_checkpoint::WorkspaceCheckpointService,
        >,
        transaction_review: Arc<crate::application::settlement::TransactionReviewService>,
        session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
        conscious_workspaces: Arc<
            crate::application::conscious_workspace::ConsciousWorkspaceRegistry,
        >,
        debug: Arc<DebugHandler>,
        session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
        recall_service: Arc<dyn mnemosyne::MemoryService>,
        memory_gateway: Arc<crate::application::memory_gateway::MemoryGatewayService>,
        memory_maintenance: Arc<
            crate::application::memory_maintenance::MemoryMaintenanceController,
        >,
        memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
        inference: Arc<dyn crate::application::inference_port::InferencePort>,
        review: Option<Arc<crate::application::governed_review::GovernedReviewService>>,
        extensions: Arc<crate::application::extension_coordinator::ExtensionCoordinator>,
        transport: Arc<TransportPorts>,
    ) -> Self {
        Self {
            kernel,
            pending_approvals,
            facts,
            goals,
            approvals,
            admin,
            sessions,
            session_lifecycle,
            health,
            _reflection: reflection,
            google,
            workflow,
            turn,
            evaluation,
            workspace_checkpoint,
            transaction_review,
            session_input,
            conscious_workspaces,
            debug,
            session_gateway,
            recall_service,
            memory_gateway,
            memory_maintenance,
            memory_health,
            inference,
            review,
            extensions,
            transport,
        }
    }
}
