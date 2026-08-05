//! Stable, transport-neutral records for non-interactive execution.

use serde::{Deserialize, Serialize};

use crate::{OperationId, TurnMetrics, TurnStop};

pub const EXEC_EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecTerminalKind {
    Completed,
    Blocked,
    Cancelled,
    ProviderUnavailable,
    ProviderRejected,
    ValidationFailed,
    OutputBackpressure,
    Failed,
}

impl ExecTerminalKind {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::Blocked => 20,
            Self::Cancelled => 21,
            Self::ProviderUnavailable => 22,
            Self::ProviderRejected => 23,
            Self::ValidationFailed => 24,
            Self::OutputBackpressure => 25,
            Self::Failed => 1,
        }
    }
}

impl From<TurnStop> for ExecTerminalKind {
    fn from(value: TurnStop) -> Self {
        match value {
            TurnStop::Completed => Self::Completed,
            TurnStop::Blocked => Self::Blocked,
            TurnStop::Cancelled => Self::Cancelled,
            TurnStop::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecEvent {
    Started,
    ActivityStarted {
        name: String,
    },
    Terminal {
        status: ExecTerminalKind,
        output: String,
        metrics: TurnMetrics,
        error_code: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecEventEnvelope {
    pub schema_version: u16,
    pub sequence: u64,
    pub session_id: String,
    pub task_id: String,
    pub turn_id: String,
    pub activity_id: Option<String>,
    pub operation_id: OperationId,
    #[serde(flatten)]
    pub event: ExecEvent,
}

impl ExecEventEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn v1(
        sequence: u64,
        session_id: impl Into<String>,
        task_id: impl Into<String>,
        turn_id: impl Into<String>,
        activity_id: Option<String>,
        operation_id: OperationId,
        event: ExecEvent,
    ) -> Self {
        Self {
            schema_version: EXEC_EVENT_SCHEMA_VERSION,
            sequence,
            session_id: session_id.into(),
            task_id: task_id.into(),
            turn_id: turn_id.into(),
            activity_id,
            operation_id,
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_envelope_has_fixed_schema_and_classified_exit_codes() {
        let envelope = ExecEventEnvelope::v1(
            1,
            "session-1",
            "task-1",
            "turn-1",
            None,
            OperationId::new(),
            ExecEvent::Terminal {
                status: ExecTerminalKind::Blocked,
                output: "approval unavailable".into(),
                metrics: TurnMetrics::default(),
                error_code: Some("approval_unavailable".into()),
            },
        );
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["sequence"], 1);
        assert_eq!(value["type"], "terminal");
        assert_eq!(value["status"], "blocked");
        assert_eq!(
            [
                ExecTerminalKind::Completed,
                ExecTerminalKind::Blocked,
                ExecTerminalKind::Cancelled,
                ExecTerminalKind::ProviderUnavailable,
                ExecTerminalKind::ProviderRejected,
                ExecTerminalKind::ValidationFailed,
                ExecTerminalKind::OutputBackpressure,
                ExecTerminalKind::Failed,
            ]
            .map(ExecTerminalKind::exit_code),
            [0, 20, 21, 22, 23, 24, 25, 1]
        );
    }
}
