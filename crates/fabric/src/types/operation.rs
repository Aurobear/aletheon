//! Operation identifiers and records shared by turn/kernel contracts.

use crate::types::time::{MonoDeadline, MonoTime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct OperationId(pub Uuid);

impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessId(pub Uuid);

impl ProcessId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProcessId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonoDeadlineMillis(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    Turn,
    ModelCall,
    CapabilityCall,
    MemoryConsolidation,
    SubAgent,
    ConsciousCycle,
    ApprovedApply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Submitted,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl OperationState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use OperationState::*;
        matches!(
            (self, next),
            (Submitted, Running)
                | (Submitted, Cancelling)
                | (Running, Cancelling)
                | (Running, Succeeded)
                | (Running, Failed)
                | (Cancelling, Cancelled)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    User,
    ParentCancelled,
    DeadlineExceeded,
    Shutdown,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationExitReason {
    Completed,
    Cancelled(CancelReason),
    Failed(String),
    Panic(String),
    DeadlineExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: OperationId,
    pub owner: ProcessId,
    pub parent: Option<OperationId>,
    pub kind: OperationKind,
    pub state: OperationState,
    pub submitted_at: MonoTime,
    pub deadline: Option<MonoDeadline>,
    pub exit: Option<OperationExitReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRequest {
    pub owner: ProcessId,
    pub parent: Option<OperationId>,
    pub kind: OperationKind,
    pub deadline: Option<MonoDeadline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub id: OperationId,
    pub state: OperationState,
    pub exit: Option<OperationExitReason>,
}
