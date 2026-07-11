//! Agent process records and lifecycle types.

use crate::types::operation::{OperationId, OperationKind};
use crate::types::time::{MonoDeadline, MonoTime, WallTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentProfileId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpaceId(pub Uuid);

impl SpaceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SpaceId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailboxId(pub Uuid);

impl MailboxId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MailboxId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamespaceId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessState {
    Created,
    Ready,
    Running,
    Waiting,
    Stopping,
    Exited,
    Failed,
}

impl ProcessState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use ProcessState::*;
        matches!(
            (self, next),
            (Created, Ready)
                | (Ready, Running)
                | (Running, Waiting)
                | (Waiting, Running)
                | (Running, Stopping)
                | (Waiting, Stopping)
                | (Stopping, Exited)
                | (Running, Failed)
                | (Waiting, Failed)
                | (Stopping, Failed)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitReason {
    Completed,
    Cancelled(String),
    Failed(String),
    Panic(String),
    DeadlineExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    pub reason: ExitReason,
    pub at: MonoTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub process_id: crate::types::operation::ProcessId,
    pub agent_id: AgentId,
    pub parent: Option<crate::types::operation::ProcessId>,
    pub profile: AgentProfileId,
    pub state: ProcessState,
    pub space: SpaceId,
    pub mailbox: MailboxId,
    pub namespace: NamespaceId,
    pub created_at: WallTime,
    pub last_heartbeat: MonoTime,
    pub exit: Option<ExitStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnSpec {
    pub agent_id: AgentId,
    pub parent: Option<crate::types::operation::ProcessId>,
    pub profile: AgentProfileId,
    pub namespace: NamespaceId,
    pub initial_operation: Option<OperationKind>,
    pub deadline: Option<MonoDeadline>,
}

impl Default for SpawnSpec {
    fn default() -> Self {
        Self {
            agent_id: AgentId::new(),
            parent: None,
            profile: AgentProfileId("default".into()),
            namespace: NamespaceId("default".into()),
            initial_operation: None,
            deadline: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessSignal {
    Start,
    Wait,
    Resume,
    Terminate,
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSnapshot {
    pub process_id: crate::types::operation::ProcessId,
    pub agent_id: AgentId,
    pub parent: Option<crate::types::operation::ProcessId>,
    pub profile: AgentProfileId,
    pub state: ProcessState,
    pub exit: Option<ExitStatus>,
    pub active_operation: Option<OperationId>,
}
