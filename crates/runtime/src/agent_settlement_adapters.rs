//! Runtime-owned Agent settlement resource adapters and admission helpers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentResourceClass, BackgroundResourceDecl,
    ReparentContext, SettlementTerminal,
};
use async_trait::async_trait;

use crate::{
    AgentAdmissionLease, AgentRunProjection, LiveAgentRun, SettlementLeasePort,
    SettlementResourcePort,
};

fn invalid(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.into(),
    }
}
pub struct RepositorySettlementLeasePort {
    repository: Arc<dyn AgentRunProjection>,
}

impl RepositorySettlementLeasePort {
    pub fn new(repository: Arc<dyn AgentRunProjection>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl SettlementLeasePort for RepositorySettlementLeasePort {
    async fn release(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError> {
        self.repository
            .delete_resource_lease(lease_key, expected_owner)
            .await
    }
}

pub struct FailClosedSettlementResourcePort {
    cancellation: tokio_util::sync::CancellationToken,
}

pub struct ManagedSettlementResourcePort {
    live: LiveAgentRun,
    parent_authority_covers: bool,
    parent_budget_accepts: ParentBudgetAcceptance,
    parent_cancellation: Option<tokio_util::sync::CancellationToken>,
    parent_mailbox_target: Option<::contracts::EnvelopeV2Target>,
}

#[derive(Clone, Default)]
struct ParentBudgetAcceptance(Arc<AtomicBool>);

impl ParentBudgetAcceptance {
    fn publish(&self, accepted: bool) {
        self.0.store(accepted, Ordering::Release);
    }

    fn read(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl ManagedSettlementResourcePort {
    pub fn new(
        live: LiveAgentRun,
        parent_authority_covers: bool,
        parent_budget_accepts: bool,
        parent_cancellation: Option<tokio_util::sync::CancellationToken>,
        parent_mailbox_target: Option<::contracts::EnvelopeV2Target>,
    ) -> Self {
        Self {
            live,
            parent_authority_covers,
            parent_budget_accepts: {
                let gate = ParentBudgetAcceptance::default();
                gate.publish(parent_budget_accepts);
                gate
            },
            parent_cancellation,
            parent_mailbox_target,
        }
    }

    pub fn set_parent_budget_accepts(&self, accepted: bool) {
        self.parent_budget_accepts.publish(accepted);
    }
}

#[async_trait]
impl SettlementResourcePort for ManagedSettlementResourcePort {
    fn reparent_context(
        &self,
        resource: &BackgroundResourceDecl,
        _parent_owner: &str,
    ) -> ReparentContext {
        ReparentContext {
            parent_authority_covers: self.parent_authority_covers
                && self.live.has_managed_resource(&resource.resource_id),
            parent_budget_accepts: self.parent_budget_accepts.read(),
            notification_route_transferable: resource.class
                != AgentResourceClass::NotificationRoute
                || self.parent_mailbox_target.is_some(),
        }
    }

    async fn settle_foreground(
        &self,
        resource: &BackgroundResourceDecl,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        if self
            .live
            .terminate_managed_resource(&resource.resource_id, action_key)
            .await
        {
            Ok(())
        } else {
            Err(invalid("managed foreground resource is unavailable"))
        }
    }

    async fn terminate(
        &self,
        resource: &BackgroundResourceDecl,
        _reason: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        if self
            .live
            .terminate_managed_resource(&resource.resource_id, action_key)
            .await
        {
            Ok(())
        } else {
            Err(invalid("managed settlement resource is unavailable"))
        }
    }

    async fn reparent(
        &self,
        resource: &BackgroundResourceDecl,
        old_owner: &str,
        new_owner: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.live
            .reparent_managed_resource(
                &resource.resource_id,
                old_owner,
                new_owner,
                action_key,
                self.parent_cancellation.as_ref(),
                self.parent_mailbox_target.as_ref(),
            )
            .await
    }
}

impl FailClosedSettlementResourcePort {
    pub fn new(cancellation: tokio_util::sync::CancellationToken) -> Self {
        Self { cancellation }
    }
}

#[async_trait]
impl SettlementResourcePort for FailClosedSettlementResourcePort {
    fn reparent_context(
        &self,
        _resource: &BackgroundResourceDecl,
        _parent_owner: &str,
    ) -> ReparentContext {
        ReparentContext {
            parent_authority_covers: false,
            parent_budget_accepts: false,
            notification_route_transferable: false,
        }
    }

    async fn settle_foreground(
        &self,
        _resource: &BackgroundResourceDecl,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.cancellation.cancel();
        Ok(())
    }

    async fn terminate(
        &self,
        _resource: &BackgroundResourceDecl,
        _reason: &str,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        self.cancellation.cancel();
        Ok(())
    }

    async fn reparent(
        &self,
        _resource: &BackgroundResourceDecl,
        _old_owner: &str,
        _new_owner: &str,
        _action_key: &str,
    ) -> Result<(), AgentControlError> {
        Err(invalid("managed resource reparent backend is unavailable"))
    }
}

pub async fn settle_admission(
    admission: &mut dyn AgentAdmissionLease,
    terminal: &SettlementTerminal,
    usage: Option<&::contracts::AttemptUsage>,
) -> Result<(), AgentControlError> {
    match (terminal, usage) {
        (SettlementTerminal::Completed, Some(usage)) => admission.settle(usage).await,
        _ => admission.revoke().await,
    }
}

pub fn terminal_with_memory_flush(
    terminal: SettlementTerminal,
    memory_error: Option<AgentControlError>,
) -> SettlementTerminal {
    match memory_error {
        Some(error) => SettlementTerminal::Failed {
            reason: format!("child memory flush failed: {}", error.message),
        },
        None => terminal,
    }
}
