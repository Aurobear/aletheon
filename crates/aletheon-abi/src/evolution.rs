//! Self-evolution loop event types.
//!
//! These events flow through the EventBus to decouple BrainCore, SelfField, and MetaRuntime.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::self_field::MutationIntent;

/// Assessment of a tool execution outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Assessment {
    Success,
    PartialSuccess,
    Failure,
}

/// A learned rule extracted from experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRule {
    pub id: Uuid,
    pub condition: String,
    pub action: String,
    pub confidence: f64,
    pub source_reflections: Vec<Uuid>,
}

/// Emitted by Engine after a tool call completes.
/// Subscribed by BrainCore for reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolObservationPayload {
    pub turn_id: Uuid,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub rules_applied: Vec<LearnedRule>,
}

/// Emitted by BrainCore after LLM reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionPayload {
    pub turn_id: Uuid,
    pub assessment: Assessment,
    pub root_cause: Option<String>,
    pub suggested_rule: Option<LearnedRule>,
    pub confidence: f64,
}

/// Emitted when BrainCore accumulates enough reflections to extract generalized rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExtractedPayload {
    pub rules: Vec<LearnedRule>,
    pub source_reflections: Vec<Uuid>,
}

/// Emitted when BrainCore detects evolution conditions are met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionTriggeredPayload {
    pub trigger_reason: String,
    pub recent_reflections: Vec<Uuid>,
    pub current_rules_snapshot: Vec<LearnedRule>,
}

/// Emitted by SelfField after validating mutation intents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationIntentPayload {
    pub intents: Vec<MutationIntent>,
    pub approved_by: String,
}

/// Emitted by MetaRuntime after Morphogenesis Pipeline completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionResultPayload {
    pub adopted: bool,
    pub genome_version_before: String,
    pub genome_version_after: Option<String>,
    pub summary: String,
}

/// LLM energy pulse — broadcast periodically by LlmPulse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitivePulseEvent {
    pub pulse_id: Uuid,
    pub timestamp: String, // ISO 8601
    pub available_tokens: u32,
    pub provider_health: ProviderHealth,
}

/// Health status of an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub name: String,
    pub available: bool,
    pub latency_ms: u64,
    pub tokens_remaining: Option<u32>,
}

/// Agent started lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStartedPayload {
    pub pid: u64,
    pub task: String,
}

/// Agent stopped lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStoppedPayload {
    pub pid: u64,
}

/// Agent spawned lifecycle event (parent -> child).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedPayload {
    pub parent: u64,
    pub child: u64,
}

/// Purpose of an LLM call, used for routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LlmPurpose {
    Reflect,
    ExtractRules,
    GenerateMutations,
    Execute,
}
