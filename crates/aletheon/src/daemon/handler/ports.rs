//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;
use serde_json::Value;

use crate::daemon::legacy_session::LegacySessionUseCases;
use crate::host::request_use_cases::{
    ExternalSourceUseCases, HealthUseCases, SessionLifecycleUseCases, TurnUseCases,
    WorkflowUseCases,
};
use crate::host::AdminUseCases;
use application::approval::ApprovalUseCases;
use application::goal::GoalUseCases;

/// Narrow consumer port for kernel connection cleanup. The daemon transport
/// needs only this lifecycle method; it must not hold the full
/// `KernelRuntime` concrete type.
#[async_trait::async_trait]
pub(crate) trait KernelCleanupPort: Send + Sync {
    async fn cleanup_disconnected_connection(
        &self,
        connection_id: &::contracts::ConnectionId,
    ) -> anyhow::Result<Vec<::contracts::ProcessId>>;
}

#[async_trait::async_trait]
impl KernelCleanupPort for kernel::KernelRuntime {
    async fn cleanup_disconnected_connection(
        &self,
        connection_id: &::contracts::ConnectionId,
    ) -> anyhow::Result<Vec<::contracts::ProcessId>> {
        kernel::KernelRuntime::cleanup_disconnected_connection(self, connection_id).await
    }
}

pub(crate) struct TransportPorts {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) capabilities_grant: corpus::ExtensionGrant,
    pub(crate) capabilities: Arc<dyn kernel::capability::governed::CapabilityService>,
    pub(crate) clock: Arc<dyn ::contracts::Clock>,
}

pub(crate) struct HandlerPorts {
    pub(crate) kernel: Arc<dyn KernelCleanupPort>,
    pub(crate) pending_approvals: Arc<dyn crate::host::admin_service::PendingApprovalsPort>,
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
    pub(crate) approvals: Arc<dyn ApprovalUseCases>,
    pub(crate) admin: Arc<dyn AdminUseCases>,
    pub(crate) sessions: Arc<dyn LegacySessionUseCases>,
    pub(crate) session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
    pub(crate) health: Arc<dyn HealthUseCases>,
    pub(crate) google: Arc<dyn ExternalSourceUseCases>,
    pub(crate) workflow: Arc<dyn WorkflowUseCases>,
    pub(crate) turn: Arc<dyn TurnUseCases>,
    pub(crate) evaluation: Arc<dyn application::evaluation::EvaluationPort>,
    pub(crate) workspace_checkpoint:
        Arc<dyn application::workspace_checkpoint::WorkspaceCheckpointPort>,
    pub(crate) transaction_review: Arc<dyn application::settlement::TransactionReviewPort>,
    pub(crate) session_input: Arc<dyn application::session_input::SessionInputPort>,
    pub(crate) conscious_workspaces:
        Arc<dyn crate::composition::conscious_workspace::ConsciousWorkspaceRegistryPort>,
    pub(crate) debug: Arc<dyn crate::daemon::debug_handler::DebugHandlerPort>,
    pub(crate) session_gateway: Arc<dyn SessionProjectionPort>,
    pub(crate) session_memory: Arc<dyn SessionMemoryProjectionPort>,
    pub(crate) recall_service: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) memory_gateway: Arc<dyn mnemosyne::memory_gateway::MemoryGatewayPort>,
    pub(crate) memory_maintenance: Arc<dyn mnemosyne::memory_maintenance::MemoryMaintenancePort>,
    pub(crate) memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
    pub(crate) inference: Arc<dyn cognit::ports::inference::InferencePort>,
    pub(crate) review: Option<Arc<dyn crate::host::governed_review::ReviewPort>>,
    pub(crate) extensions: Arc<dyn crate::extensions::ExtensionsPort>,
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
        kernel: Arc<dyn KernelCleanupPort>,
        pending_approvals: Arc<dyn crate::host::admin_service::PendingApprovalsPort>,
        facts: Arc<dyn FactUseCases>,
        goals: Arc<dyn GoalUseCases>,
        approvals: Arc<dyn ApprovalUseCases>,
        admin: Arc<dyn AdminUseCases>,
        sessions: Arc<dyn LegacySessionUseCases>,
        session_lifecycle: Arc<dyn SessionLifecycleUseCases>,
        health: Arc<dyn HealthUseCases>,
        google: Arc<dyn ExternalSourceUseCases>,
        workflow: Arc<dyn WorkflowUseCases>,
        turn: Arc<dyn TurnUseCases>,
        evaluation: Arc<dyn application::evaluation::EvaluationPort>,
        workspace_checkpoint: Arc<dyn application::workspace_checkpoint::WorkspaceCheckpointPort>,
        transaction_review: Arc<dyn application::settlement::TransactionReviewPort>,
        session_input: Arc<dyn application::session_input::SessionInputPort>,
        conscious_workspaces: Arc<
            dyn crate::composition::conscious_workspace::ConsciousWorkspaceRegistryPort,
        >,
        debug: Arc<dyn crate::daemon::debug_handler::DebugHandlerPort>,
        session_gateway: Arc<dyn SessionProjectionPort>,
        session_memory: Arc<dyn SessionMemoryProjectionPort>,
        recall_service: Arc<dyn mnemosyne::MemoryService>,
        memory_gateway: Arc<dyn mnemosyne::memory_gateway::MemoryGatewayPort>,
        memory_maintenance: Arc<dyn mnemosyne::memory_maintenance::MemoryMaintenancePort>,
        memory_health: Arc<std::sync::Mutex<mnemosyne::CompositeMemoryHealth>>,
        inference: Arc<dyn cognit::ports::inference::InferencePort>,
        review: Option<Arc<dyn crate::host::governed_review::ReviewPort>>,
        extensions: Arc<dyn crate::extensions::ExtensionsPort>,
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
