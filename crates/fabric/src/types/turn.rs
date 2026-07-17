//! Turn request/result contracts used by adapters and execution services.

use super::local_authority::PrincipalContext;
use crate::types::operation::{MonoDeadlineMillis, OperationId, ProcessId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub context: PrincipalContext,
    pub input: String,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TurnStop {
    Completed,
    Blocked,
    Cancelled,
    Failed,
}

/// Authoritative terminal status exposed by the versioned client protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalStatus {
    Completed,
    Failed,
    Interrupted,
}

impl From<TurnStop> for TurnTerminalStatus {
    fn from(value: TurnStop) -> Self {
        match value {
            TurnStop::Completed => Self::Completed,
            TurnStop::Cancelled => Self::Interrupted,
            TurnStop::Blocked | TurnStop::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    pub output: String,
    pub stop: TurnStop,
    pub metrics: TurnMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TurnEvent {
    Started {
        operation_id: OperationId,
    },
    Finished {
        operation_id: OperationId,
        stop: TurnStop,
    },
    ToolCall {
        operation_id: OperationId,
        name: String,
    },
}
