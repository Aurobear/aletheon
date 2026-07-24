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
    pub(crate) reflection: Arc<dyn ReflectionUseCases>,
    pub(crate) google: Arc<dyn ExternalSourceUseCases>,
    pub(crate) workflow: Arc<dyn WorkflowUseCases>,
    pub(crate) turn: Arc<dyn TurnUseCases>,
    pub(crate) session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
    pub(crate) conscious_workspaces:
        Arc<crate::application::conscious_workspace::ConsciousWorkspaceRegistry>,
    pub(crate) debug: Arc<DebugHandler>,
    pub(crate) session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
    pub(crate) recall_service: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
    pub(crate) transport: Arc<TransportPorts>,
}

impl HandlerPorts {
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
        session_input: Arc<crate::application::session_input::SessionInputCoordinator>,
        conscious_workspaces: Arc<
            crate::application::conscious_workspace::ConsciousWorkspaceRegistry,
        >,
        debug: Arc<DebugHandler>,
        session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
        recall_service: Arc<dyn mnemosyne::MemoryService>,
        memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
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
            reflection,
            google,
            workflow,
            turn,
            session_input,
            conscious_workspaces,
            debug,
            session_gateway,
            recall_service,
            memory_health,
            transport,
        }
    }
}
