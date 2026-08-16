//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;
use serde_json::Value;

use crate::wiring::approval_service::ApprovalUseCases;
use crate::wiring::daemon::debug_handler::DebugHandler;
use crate::wiring::daemon::legacy_session::LegacySessionUseCases;
use crate::wiring::application::request_use_cases::{
    ExternalSourceUseCases, HealthUseCases, ReflectionUseCases, SessionLifecycleUseCases,
    TurnUseCases, WorkflowUseCases,
};
use crate::wiring::application::{AdminUseCases, GoalUseCases};

pub(crate) struct TransportPorts {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) capabilities_grant: corpus::ExtensionGrant,
    pub(crate) capabilities:
        Arc<dyn kernel::capability::governed::CapabilityService>,
    pub(crate) clock: Arc<dyn ::contracts::Clock>,
}

pub(crate) struct HandlerPorts {
    pub(crate) kernel: Arc<kernel::KernelRuntime>,
    pub(crate) pending_approvals: crate::wiring::application::admin_service::PendingApprovals,
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
    pub(crate) evaluation: Arc<crate::wiring::application::evaluation::EvaluationService>,
    pub(crate) workspace_checkpoint:
        Arc<crate::wiring::application::workspace_checkpoint::WorkspaceCheckpointService>,
    pub(crate) transaction_review: Arc<application::settlement::TransactionReviewService>,
    pub(crate) session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub(crate) conscious_workspaces:
        Arc<crate::wiring::application::conscious_workspace::ConsciousWorkspaceRegistry>,
    pub(crate) debug: Arc<DebugHandler>,
    pub(crate) session_gateway: Arc<dyn SessionProjectionPort>,
    pub(crate) session_memory: Arc<dyn SessionMemoryProjectionPort>,
    pub(crate) recall_service: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) memory_gateway: Arc<mnemosyne::MemoryGatewayService>,
    pub(crate) memory_maintenance: Arc<mnemosyne::memory_maintenance::MemoryMaintenanceController>,
    pub(crate) memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
    pub(crate) inference: Arc<dyn cognit::ports::inference::InferencePort>,
    pub(crate) review: Option<Arc<crate::wiring::governed_review::GovernedReviewService>>,
    pub(crate) extensions: Arc<crate::extensions::extension_coordinator::ExtensionCoordinator>,
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
        pending_approvals: crate::wiring::application::admin_service::PendingApprovals,
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
        evaluation: Arc<crate::wiring::application::evaluation::EvaluationService>,
        workspace_checkpoint: Arc<
            crate::wiring::application::workspace_checkpoint::WorkspaceCheckpointService,
        >,
        transaction_review: Arc<application::settlement::TransactionReviewService>,
        session_input: Arc<application::session_input::SessionInputCoordinator>,
        conscious_workspaces: Arc<
            crate::wiring::application::conscious_workspace::ConsciousWorkspaceRegistry,
        >,
        debug: Arc<DebugHandler>,
        session_gateway: Arc<dyn SessionProjectionPort>,
        session_memory: Arc<dyn SessionMemoryProjectionPort>,
        recall_service: Arc<dyn mnemosyne::MemoryService>,
        memory_gateway: Arc<mnemosyne::MemoryGatewayService>,
        memory_maintenance: Arc<mnemosyne::memory_maintenance::MemoryMaintenanceController>,
        memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
        inference: Arc<dyn cognit::ports::inference::InferencePort>,
        review: Option<Arc<crate::wiring::governed_review::GovernedReviewService>>,
        extensions: Arc<crate::extensions::extension_coordinator::ExtensionCoordinator>,
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
            session_memory,
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

/// Gateway read/stream operations consumed by typed daemon routes.  The
/// compatibility Aletheon SessionGateway implements this adapter today; the
/// production handler no longer owns that concrete core type.
#[async_trait::async_trait]
pub(crate) trait SessionProjectionPort: Send + Sync {
    async fn protocol_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::UiSnapshot>;

    async fn protocol_snapshot(
        &self,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::UiSnapshot>;

    async fn protocol_read_snapshot(
        &self,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot>;

    async fn protocol_read_snapshot_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot>;

    async fn protocol_event_page_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> anyhow::Result<::contracts::protocol::client::SessionEventPage>;

    async fn protocol_session_list_for(
        &self,
        principal: &::contracts::PrincipalId,
    ) -> anyhow::Result<::contracts::protocol::client::SessionListSnapshot>;

    async fn protocol_events_after_for(
        &self,
        principal: &::contracts::PrincipalId,
        session_id: &::contracts::SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> anyhow::Result<Vec<::contracts::protocol::client::ClientEvent>>;
}

/// Canonical read-only memory snapshot used by the typed Gateway query.
///
/// This is separate from the legacy `session.*` debug dispatcher so a
/// versioned client never depends on the Aletheon SessionGateway facade.
#[async_trait::async_trait]
pub(crate) trait SessionMemoryProjectionPort: Send + Sync {
    async fn snapshot(&self, memory_type: &str, limit: usize) -> anyhow::Result<Value>;
}
