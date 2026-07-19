//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

use crate::service::legacy_session_service::LegacySessionUseCases;
use crate::service::request_use_cases::{
    DebugUseCases, GoogleUseCases, HealthUseCases, ReflectionUseCases, SessionLifecycleUseCases,
    TurnUseCases, WorkflowUseCases,
};
use crate::service::{AdminUseCases, ApprovalUseCases, GoalUseCases};

pub(crate) struct TransportPorts {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) capabilities_grant: corpus::ExtensionGrant,
    pub(crate) capabilities: Arc<dyn crate::service::governed_capability::CapabilityService>,
    pub(crate) clock: Arc<dyn fabric::Clock>,
}

pub(crate) struct HandlerPorts {
    pub(crate) kernel: Arc<kernel::KernelRuntime>,
    pub(crate) pending_approvals: crate::service::admin_service::PendingApprovals,
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
    pub(crate) approvals: Arc<dyn ApprovalUseCases>,
    pub(crate) admin: Arc<dyn AdminUseCases>,
    pub(crate) sessions: Arc<dyn LegacySessionUseCases>,
    pub(crate) session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
    pub(crate) health: Arc<dyn HealthUseCases>,
    pub(crate) reflection: Arc<dyn ReflectionUseCases>,
    pub(crate) google: Arc<dyn GoogleUseCases>,
    pub(crate) workflow: Arc<dyn WorkflowUseCases>,
    pub(crate) turn: Arc<dyn TurnUseCases>,
    pub(crate) session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
    pub(crate) conscious_workspaces:
        Arc<crate::service::conscious_workspace::ConsciousWorkspaceRegistry>,
    pub(crate) debug: Arc<dyn DebugUseCases>,
    pub(crate) session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
    pub(crate) transport: Arc<TransportPorts>,
}

impl HandlerPorts {
    pub(crate) fn new(
        kernel: Arc<kernel::KernelRuntime>,
        pending_approvals: crate::service::admin_service::PendingApprovals,
        facts: Arc<dyn FactUseCases>,
        goals: Arc<dyn GoalUseCases>,
        approvals: Arc<dyn ApprovalUseCases>,
        admin: Arc<dyn AdminUseCases>,
        sessions: Arc<dyn LegacySessionUseCases>,
        session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
        health: Arc<dyn HealthUseCases>,
        reflection: Arc<dyn ReflectionUseCases>,
        google: Arc<dyn GoogleUseCases>,
        workflow: Arc<dyn WorkflowUseCases>,
        turn: Arc<dyn TurnUseCases>,
        session_input: Arc<crate::service::session_input::SessionInputCoordinator>,
        conscious_workspaces: Arc<crate::service::conscious_workspace::ConsciousWorkspaceRegistry>,
        debug: Arc<dyn DebugUseCases>,
        session_gateway: Arc<crate::core::session_gateway::SessionGateway>,
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
            transport,
        }
    }
}
