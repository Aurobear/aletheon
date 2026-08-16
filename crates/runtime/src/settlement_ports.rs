//! Runtime-owned settlement resource and lease ports.

use ::contracts::{AgentControlError, BackgroundResourceDecl, ReparentContext};
use async_trait::async_trait;

/// Runtime quiesce boundary used before irreversible child settlement.
/// Host live-run implementations provide the resource snapshot; Runtime owns
/// the ordering fence and subsequent settlement decision.
#[async_trait]
pub trait SettlementQuiescePort: Send + Sync {
    async fn begin_quiescing(&self) -> Vec<BackgroundResourceDecl>;
}

/// Host resource operations required by Runtime settlement ordering.
#[async_trait]
pub trait SettlementResourcePort: Send + Sync {
    fn reparent_context(
        &self,
        resource: &BackgroundResourceDecl,
        parent_owner: &str,
    ) -> ReparentContext;

    async fn settle_foreground(
        &self,
        resource: &BackgroundResourceDecl,
        action_key: &str,
    ) -> Result<(), AgentControlError>;

    async fn terminate(
        &self,
        resource: &BackgroundResourceDecl,
        reason: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError>;

    async fn reparent(
        &self,
        resource: &BackgroundResourceDecl,
        old_owner: &str,
        new_owner: &str,
        action_key: &str,
    ) -> Result<(), AgentControlError>;
}

/// Host lease deletion port used by Runtime settlement.
#[async_trait]
pub trait SettlementLeasePort: Send + Sync {
    async fn release(
        &self,
        lease_key: &str,
        expected_owner: &str,
    ) -> Result<bool, AgentControlError>;
}
