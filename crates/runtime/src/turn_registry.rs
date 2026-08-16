//! Runtime-owned active Turn identity and cancellation records (RA-04).

use crate::{TurnCancellation, TurnId};
use ::contracts::{
    CancelReason, ConnectionId, MonoDeadline, MonoTime, OperationId, PrincipalId, ThreadId,
};
use std::collections::HashMap;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Clone)]
pub struct ActiveTurn {
    pub operation_id: OperationId,
    /// Compatibility wire spelling used by Fabric/session projections.
    pub turn_id: ::contracts::TurnId,
    /// Runtime-owned canonical identity; never minted by the host record.
    pub canonical_turn_id: TurnId,
    pub connection_id: ConnectionId,
    pub cancellation: TurnCancellation,
    pub started_at: MonoTime,
    pub deadline_at: Option<MonoDeadline>,
}

impl ActiveTurn {
    pub fn cancel(&self, reason: CancelReason) {
        self.cancellation.request(reason);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.token().is_cancelled()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ActiveTurnKey {
    pub principal_id: PrincipalId,
    pub thread_id: ThreadId,
}

impl ActiveTurnKey {
    pub fn new(principal_id: PrincipalId, thread_id: ThreadId) -> Self {
        Self {
            principal_id,
            thread_id,
        }
    }

    pub fn from_context(context: &::contracts::PrincipalContext) -> Self {
        Self::new(context.principal_id.clone(), context.thread_id.clone())
    }
}

/// Runtime-owned live Turn registry shared with host composition and Session
/// command/query adapters.
///
/// Keeping the map here prevents an outer application coordinator from
/// becoming a second Turn lifecycle authority. The transitional `lock` method
/// preserves atomic admission/insert operations while callers migrate toward
/// narrower Runtime commands.
#[derive(Default)]
pub struct ActiveTurnRegistry {
    active: Mutex<HashMap<ActiveTurnKey, ActiveTurn>>,
}

impl ActiveTurnRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn lock(&self) -> MutexGuard<'_, HashMap<ActiveTurnKey, ActiveTurn>> {
        self.active.lock().await
    }
}
