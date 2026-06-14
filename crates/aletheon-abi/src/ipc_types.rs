//! IPC trait and message types.
//!
//! Merged from argos-types ipc module into aletheon-abi.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub type AgentId = u64;

/// Message type discriminator for IPC routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Direct,
    Event,
    ToolCall,
    Control,
}

/// Priority levels for IPC message ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcPriority {
    Urgent,
    ToolCall,
    Normal,
    Background,
    Batch,
}

/// Wire-format message exchanged over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub sender_id: AgentId,
    pub target_id: AgentId,
    pub msg_type: MessageType,
    pub priority: IpcPriority,
    pub payload: Vec<u8>,
}

impl AgentMessage {
    pub fn new(
        sender_id: AgentId,
        target_id: AgentId,
        msg_type: MessageType,
        priority: IpcPriority,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            sender_id,
            target_id,
            msg_type,
            priority,
            payload,
        }
    }

    pub fn event(
        sender_id: AgentId,
        target_id: AgentId,
        priority: IpcPriority,
        payload: &[u8],
    ) -> Self {
        Self {
            sender_id,
            target_id,
            msg_type: MessageType::Event,
            priority,
            payload: payload.to_vec(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("AgentMessage serialization should not fail")
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}

/// IPC transport preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcPreference {
    UnixSocket,
    IoUring,
    SharedMemory,
    Auto,
}

/// Error from IPC backend probing.
#[derive(Debug, Clone)]
pub enum IpcProbeError {
    NotSupported,
    PermissionDenied,
    ResourceExhausted,
    Other(String),
}

impl std::fmt::Display for IpcProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupported => write!(f, "IPC backend not supported"),
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::ResourceExhausted => write!(f, "resource exhausted"),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for IpcProbeError {}

/// Trait for IPC backends.
#[async_trait]
pub trait IpcBackend: Send + Sync {
    async fn init(&mut self) -> Result<(), IpcProbeError>;
    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError>;
    async fn recv(&self) -> Result<AgentMessage, IpcProbeError>;
    async fn try_recv(&self) -> Option<AgentMessage>;
    fn is_available(&self) -> bool;
    fn name(&self) -> &str;
}
