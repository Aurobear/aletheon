//! Runtime owner ports (RA-01).
//!
//! Narrow ports the Runtime exposes to its composition root and to owners.
//! These define contract/port only — nothing appends, spawns, or cuts over a
//! legacy writer in this seam.

use crate::command::{CommandReceipt, RuntimeCommand};
use crate::error::RuntimeError;
use crate::event::{RuntimeEvent, TurnTerminal};
use crate::ids::{SessionId, TurnId};
use crate::query::RuntimeQuery;
use crate::AgentProfileDocument;
use async_trait::async_trait;

/// Durable sink for Runtime-owned turn lifecycle events. The sink is injected
/// by the composition root; the Runtime writer never guesses terminal state
/// from transport or model output.
#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn append(&self, event: crate::event::TurnStreamEvent) -> Result<(), RuntimeError>;
}

/// Narrow lifecycle port consumed by host execution façades.  The concrete
/// `RuntimeTurnWriter` remains the Runtime owner; callers cannot depend on its
/// reducer, replay maps, or event-sink implementation details.
#[async_trait]
pub trait TurnLifecycleWriter: Send + Sync {
    async fn start_turn(&self, session: &SessionId) -> Result<TurnId, RuntimeError>;
    async fn settle(
        &self,
        session: &SessionId,
        turn: &TurnId,
        terminal: TurnTerminal,
    ) -> Result<(), RuntimeError>;
    async fn cancel(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<crate::turn_reducer::TransitionOutcome, RuntimeError>;
    async fn timeout(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<crate::turn_reducer::TransitionOutcome, RuntimeError>;
    async fn disconnect(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<crate::turn_reducer::TransitionOutcome, RuntimeError>;
    async fn late_receipt(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<crate::turn_reducer::TransitionOutcome, RuntimeError>;
    async fn crash(
        &self,
        session: &SessionId,
        turn: &TurnId,
    ) -> Result<crate::turn_reducer::TransitionOutcome, RuntimeError>;
}

/// Durable sink for Runtime-owned AgentRun lifecycle events.  AgentRun
/// bindings are process-local handles, but their admission and terminal
/// receipts must be journaled so a supervisor can reconcile them after a
/// restart instead of treating an empty binding map as success.
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn append(&self, event: crate::event::AgentStreamEvent) -> Result<(), RuntimeError>;
}

/// Runtime profile authority. Filesystem/Markdown parsing belongs to a host
/// adapter; consumers depend on this semantic port instead of a loader name.
pub trait AgentProfilePort: Send + Sync {
    fn profiles(&self) -> Vec<AgentProfileDocument>;
}

/// Runtime command boundary.  Implemented by the Runtime composition root;
/// consumed by adapters (Gateway/ACP/TUI) that translate caller requests.
#[async_trait]
pub trait RuntimeCommandPort: Send + Sync {
    async fn dispatch(&self, command: RuntimeCommand) -> Result<CommandReceipt, RuntimeError>;
}

/// Runtime query boundary.
#[async_trait]
pub trait RuntimeQueryPort: Send + Sync {
    async fn query(&self, query: RuntimeQuery) -> Result<serde_json::Value, RuntimeError>;
}

/// Runtime event subscription boundary (snapshot + cursor recovery).
#[async_trait]
pub trait RuntimeEventPort: Send + Sync {
    async fn next_event(&self) -> Result<RuntimeEvent, RuntimeError>;
}
