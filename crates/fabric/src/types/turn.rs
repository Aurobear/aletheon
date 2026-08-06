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
    /// Explicit host/client workflow obligations for this turn. These are
    /// protocol data and must never be inferred by matching prompt text.
    #[serde(default)]
    pub requirements: Vec<crate::TurnRequirement>,
    /// Explicit client/workflow task semantics. Never inferred from `input`.
    #[serde(default)]
    pub requested_task_kind: Option<crate::TaskKind>,
    /// Host-issued evaluation contract. Clients cannot author this field.
    #[serde(default)]
    pub evaluation_contract: Option<crate::TaskEvaluationContract>,
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
    #[serde(default)]
    pub provider_retries: u64,
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
    /// Canonical embodied-skill progress event. Carries the real operation id
    /// injected by the bounded progress sink — never a provider-generated string.
    EmbodimentProgress {
        operation_id: OperationId,
        skill: String,
        fraction: f32,
        note: String,
    },
    /// Immutable robot terminal receipt emitted only after the domain episode
    /// sink has accepted it. Executive projects this typed receipt into the
    /// canonical Session/Task/Activity read model; clients never parse the
    /// rendered assistant JSON to recover robot authority facts.
    RobotEpisodeSettled {
        receipt: Box<crate::types::episode_report::SettledEpisodeReport>,
    },
}

#[cfg(test)]
mod tests {
    use super::TurnMetrics;

    #[test]
    fn legacy_turn_metrics_default_provider_retries_without_losing_new_values() {
        let legacy = serde_json::json!({
            "tool_calls_made": 2,
            "tool_errors": 0,
            "elapsed_ms": 10,
            "iterations": 1,
            "completed_normally": true
        });
        let decoded: TurnMetrics = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.provider_retries, 0);

        let current = TurnMetrics {
            provider_retries: 3,
            ..decoded
        };
        let round_trip: TurnMetrics =
            serde_json::from_value(serde_json::to_value(current).unwrap()).unwrap();
        assert_eq!(round_trip.provider_retries, 3);
    }
}
