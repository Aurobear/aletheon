//! Envelope V2 — unified message envelope for the Communication Fabric V2.
//!
//! This is the Phase 4A migration type: it coexists with the legacy `Envelope`
//! in `crate::ipc::envelope`. New code should target `EnvelopeV2`; old code
//! continues to use `Envelope` until PR-4E decommissions it.
//!
//! Design: docs/arch/04_COMMUNICATION_FABRIC_V2.md

use crate::types::operation::{MonoDeadlineMillis, OperationId};
use crate::types::process::NamespaceId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Newtype identifiers
// ---------------------------------------------------------------------------

/// Unique message identifier (UUID v4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable schema identifier for cross-process payload compatibility.
///
/// Every cross-process payload MUST carry an explicit schema (e.g.
/// `"aletheon.turn.request/v1"`). Unknown schemas are rejected with a
/// structured error — there is no silent JSON-guessing fallback.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaId(pub String);

impl SchemaId {
    /// Create a `SchemaId` from a known schema string.
    pub const fn new(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for SchemaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for SchemaId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SchemaId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Structured schema compatibility error. Unknown cross-process schemas are
/// rejected instead of being interpreted as untyped JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedSchema {
    pub schema: SchemaId,
    pub supported: Vec<String>,
}

impl std::fmt::Display for UnsupportedSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported schema: {}", self.schema)
    }
}

impl std::error::Error for UnsupportedSchema {}

// ---------------------------------------------------------------------------
// Routing target (string-based, simpler than the legacy enum)
// ---------------------------------------------------------------------------

/// Routing target — a plain string identifier.
///
/// Unlike the legacy `Target` enum (which encoded Module/Agent/Topic/Broadcast
/// as variants), V2 uses a flat string so transports and resolvers are not
/// coupled to the in-process module taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target(pub String);

impl Target {
    pub const fn new(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for Target {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Target {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Delivery pattern
// ---------------------------------------------------------------------------

/// Communication pattern — determines delivery semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeliveryPattern {
    /// Point-to-point delivery to exactly one recipient.
    Direct,
    /// Broadcast to all subscribers of a topic.
    FanOut,
    /// Request that expects exactly one correlated response.
    RequestResponse,
}

// ---------------------------------------------------------------------------
// Envelope V2
// ---------------------------------------------------------------------------

/// Unified message envelope V2 — wire format for all communication.
///
/// Replaces the legacy `Envelope` (Phase 4 migration).
/// Every field is intentional; there is no `timestamp_ms` (use `logical_time`
/// for ordering and `deadline` for expiry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeV2 {
    /// Unique message identifier.
    pub id: MessageId,
    /// Stable schema identifier (e.g. `"aletheon.turn.request/v1"`).
    pub schema: SchemaId,
    /// Sender identity.
    pub source: Target,
    /// Receiver identity.
    pub target: Target,
    /// Delivery pattern.
    pub pattern: DeliveryPattern,
    /// Operation this message belongs to (if any).
    pub operation_id: Option<OperationId>,
    /// Message that directly caused this one (happens-before chain).
    pub causation_id: Option<MessageId>,
    /// Correlation ID for request-response pairing.
    pub correlation_id: Option<MessageId>,
    /// Logical namespace (tenant, space, domain).
    pub namespace: NamespaceId,
    /// Logical clock value — monotonically increasing per-namespace.
    pub logical_time: u64,
    /// Deadline in monotonic milliseconds. `None` = no expiry.
    pub deadline: Option<MonoDeadlineMillis>,
    /// Priority (0 = lowest, 255 = highest).
    pub priority: u8,
    /// Structured JSON payload.
    pub payload: serde_json::Value,
}

impl EnvelopeV2 {
    /// Create a new `EnvelopeV2` with a fresh `MessageId` and logical time of 0.
    pub fn new(
        schema: SchemaId,
        source: Target,
        target: Target,
        pattern: DeliveryPattern,
        namespace: NamespaceId,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: MessageId::new(),
            schema,
            source,
            target,
            pattern,
            operation_id: None,
            causation_id: None,
            correlation_id: None,
            namespace,
            logical_time: 0,
            deadline: None,
            priority: 128,
            payload,
        }
    }

    /// Set the logical time.
    pub fn with_logical_time(mut self, t: u64) -> Self {
        self.logical_time = t;
        self
    }

    /// Set the operation ID.
    pub fn with_operation_id(mut self, id: OperationId) -> Self {
        self.operation_id = Some(id);
        self
    }

    /// Set the causation ID (happens-before).
    pub fn with_causation_id(mut self, id: MessageId) -> Self {
        self.causation_id = Some(id);
        self
    }

    /// Set the correlation ID (request-response).
    pub fn with_correlation_id(mut self, id: MessageId) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set the deadline.
    pub fn with_deadline(mut self, d: MonoDeadlineMillis) -> Self {
        self.deadline = Some(d);
        self
    }

    /// Set the priority (0-255).
    pub fn with_priority(mut self, p: u8) -> Self {
        self.priority = p;
        self
    }

    /// Check whether this envelope has expired at the given wall-clock
    /// timestamp (milliseconds since epoch).
    ///
    /// `now` should be a monotonic clock value in milliseconds.
    /// Returns `false` when there is no deadline set.
    pub fn is_expired_at(&self, now_mono_millis: u64) -> bool {
        match self.deadline {
            Some(MonoDeadlineMillis(d)) => now_mono_millis >= d,
            None => false,
        }
    }

    pub fn validate_known_schema(&self) -> Result<(), UnsupportedSchema> {
        self.schema.validate_known()
    }
}

// ---------------------------------------------------------------------------
// Well-known schema constants
// ---------------------------------------------------------------------------

impl SchemaId {
    pub const TURN_REQUEST_V1: &'static str = "aletheon.turn.request/v1";
    pub const TURN_EVENT_V1: &'static str = "aletheon.turn.event/v1";
    pub const PROCESS_SIGNAL_V1: &'static str = "aletheon.process.signal/v1";
    pub const CAPABILITY_REQUEST_V1: &'static str = "aletheon.capability.request/v1";
    pub const CAPABILITY_RESULT_V1: &'static str = "aletheon.capability.result/v1";
    pub const AGENT_CONTROL_MESSAGE_V1: &'static str = "aletheon.agent.control-message/v1";

    // -- Migration bridge: each legacy EventType variant maps to a SchemaId --
    // User-space
    pub const EVENT_USER_INTENT_V1: &'static str = "aletheon.event.user_intent/v1";
    pub const EVENT_USER_FEEDBACK_V1: &'static str = "aletheon.event.user_feedback/v1";
    // Environment
    pub const EVENT_ENVIRONMENT_CHANGE_V1: &'static str = "aletheon.event.environment_change/v1";
    pub const EVENT_PERCEPTION_UPDATE_V1: &'static str = "aletheon.event.perception_update/v1";
    // BodyRuntime
    pub const EVENT_TOOL_OBSERVATION_V1: &'static str = "aletheon.event.tool_observation/v1";
    pub const EVENT_TOOL_ERROR_V1: &'static str = "aletheon.event.tool_error/v1";
    pub const EVENT_ACTION_COMPLETED_V1: &'static str = "aletheon.event.action_completed/v1";
    // Memory
    pub const EVENT_MEMORY_STORED_V1: &'static str = "aletheon.event.memory_stored/v1";
    pub const EVENT_MEMORY_RECALLED_V1: &'static str = "aletheon.event.memory_recalled/v1";
    pub const EVENT_MEMORY_COMPACTED_V1: &'static str = "aletheon.event.memory_compacted/v1";
    // SelfField
    pub const EVENT_IDENTITY_QUERY_V1: &'static str = "aletheon.event.identity_query/v1";
    pub const EVENT_BOUNDARY_CHECK_V1: &'static str = "aletheon.event.boundary_check/v1";
    pub const EVENT_CONFLICT_DETECTED_V1: &'static str = "aletheon.event.conflict_detected/v1";
    pub const EVENT_REJECTION_ISSUED_V1: &'static str = "aletheon.event.rejection_issued/v1";
    // CognitCore
    pub const EVENT_PLAN_GENERATED_V1: &'static str = "aletheon.event.plan_generated/v1";
    pub const EVENT_REFLECTION_COMPLETE_V1: &'static str = "aletheon.event.reflection_complete/v1";
    pub const EVENT_CRITICISM_RAISED_V1: &'static str = "aletheon.event.criticism_raised/v1";
    // MetaRuntime
    pub const EVENT_MUTATION_INTENT_V1: &'static str = "aletheon.event.mutation_intent/v1";
    pub const EVENT_RUNTIME_CANDIDATE_V1: &'static str = "aletheon.event.runtime_candidate/v1";
    pub const EVENT_MIGRATION_STARTED_V1: &'static str = "aletheon.event.migration_started/v1";
    pub const EVENT_MIGRATION_COMPLETE_V1: &'static str = "aletheon.event.migration_complete/v1";
    // Lifecycle
    pub const EVENT_SUBSYSTEM_STARTED_V1: &'static str = "aletheon.event.subsystem_started/v1";
    pub const EVENT_SUBSYSTEM_FAILED_V1: &'static str = "aletheon.event.subsystem_failed/v1";
    pub const EVENT_HEALTH_CHECK_V1: &'static str = "aletheon.event.health_check/v1";
    // Runtime
    pub const EVENT_AGENT_STARTED_V1: &'static str = "aletheon.event.agent_started/v1";
    pub const EVENT_AGENT_STOPPED_V1: &'static str = "aletheon.event.agent_stopped/v1";
    pub const EVENT_AGENT_FAILED_V1: &'static str = "aletheon.event.agent_failed/v1";
    pub const EVENT_SCHEDULED_TASK_FIRED_V1: &'static str =
        "aletheon.event.scheduled_task_fired/v1";
    pub const EVENT_BOOT_PHASE_CHANGED_V1: &'static str = "aletheon.event.boot_phase_changed/v1";
    pub const EVENT_REACT_ITERATION_START_V1: &'static str =
        "aletheon.event.react_iteration_start/v1";
    pub const EVENT_REACT_ITERATION_END_V1: &'static str = "aletheon.event.react_iteration_end/v1";
    pub const EVENT_AGENT_FORK_COMPLETED_V1: &'static str =
        "aletheon.event.agent_fork_completed/v1";
    // Self-evolution
    pub const EVENT_RULE_EXTRACTED_V1: &'static str = "aletheon.event.rule_extracted/v1";
    pub const EVENT_EVOLUTION_TRIGGERED_V1: &'static str = "aletheon.event.evolution_triggered/v1";
    pub const EVENT_EVOLUTION_RESULT_V1: &'static str = "aletheon.event.evolution_result/v1";
    // Energy / agent lifecycle
    pub const EVENT_COGNITIVE_PULSE_V1: &'static str = "aletheon.event.cognitive_pulse/v1";
    pub const EVENT_AGENT_SPAWNED_V1: &'static str = "aletheon.event.agent_spawned/v1";
    pub const EVENT_MEMORY_CANDIDATE_V1: &'static str = "aletheon.event.memory_candidate/v1";
    pub const EVENT_AGORA_BROADCAST_V1: &'static str = "aletheon.event.agora_broadcast/v1";
    pub const EVENT_RUNTIME_RESTART_V1: &'static str = "aletheon.event.runtime_restart/v1";

    pub fn supported_cross_process() -> &'static [&'static str] {
        &[
            Self::TURN_REQUEST_V1,
            Self::TURN_EVENT_V1,
            Self::PROCESS_SIGNAL_V1,
            Self::CAPABILITY_REQUEST_V1,
            Self::CAPABILITY_RESULT_V1,
            Self::AGENT_CONTROL_MESSAGE_V1,
        ]
    }

    pub fn is_supported_cross_process(&self) -> bool {
        Self::supported_cross_process().contains(&self.0.as_str())
            || self.0.starts_with("aletheon.event.")
            || self.0.starts_with("aletheon.test")
    }

    pub fn validate_known(&self) -> Result<(), UnsupportedSchema> {
        if self.is_supported_cross_process() {
            Ok(())
        } else {
            Err(UnsupportedSchema {
                schema: self.clone(),
                supported: Self::supported_cross_process()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::process::NamespaceId;

    #[test]
    fn envelope_v2_serializes_and_deserializes() {
        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.turn.request/v1"),
            Target::from("executive"),
            Target::from("cognit"),
            DeliveryPattern::Direct,
            NamespaceId("test-ns".into()),
            serde_json::json!({"prompt": "hello"}),
        )
        .with_priority(200)
        .with_logical_time(42);

        let json = serde_json::to_string(&env).expect("serialize");
        let env2: EnvelopeV2 = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(env.id, env2.id);
        assert_eq!(env.schema, env2.schema);
        assert_eq!(env.source, env2.source);
        assert_eq!(env.target, env2.target);
        assert_eq!(env.pattern, env2.pattern);
        assert_eq!(env.namespace, env2.namespace);
        assert_eq!(env.logical_time, env2.logical_time);
        assert_eq!(env.priority, env2.priority);
        assert_eq!(env.payload, env2.payload);
    }

    #[test]
    fn envelope_v2_request_response_correlation() {
        let req = EnvelopeV2::new(
            SchemaId::from("aletheon.capability.request/v1"),
            Target::from("agent-1"),
            Target::from("agent-2"),
            DeliveryPattern::RequestResponse,
            NamespaceId("test".into()),
            serde_json::json!({"ask": "ping"}),
        );

        let resp = EnvelopeV2::new(
            SchemaId::from("aletheon.capability.result/v1"),
            Target::from("agent-2"),
            Target::from("agent-1"),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!({"answer": "pong"}),
        )
        .with_correlation_id(req.id);

        assert_eq!(resp.correlation_id, Some(req.id));
    }

    #[test]
    fn schema_unknown_is_not_silent() {
        // The SchemaId is explicit — deserializing a payload without a schema
        // field MUST fail. There is no "guess" fallback.
        let json_without_schema = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "source": "test",
            "target": "test",
            "pattern": "Direct",
            "namespace": "test",
            "logical_time": 0,
            "priority": 128,
            "payload": {}
        });

        let result: Result<EnvelopeV2, _> = serde_json::from_value(json_without_schema);
        assert!(
            result.is_err(),
            "deserializing an EnvelopeV2 without 'schema' must fail"
        );

        // Conversely, with a schema it works.
        let json_with_schema = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000002",
            "schema": "aletheon.turn.request/v1",
            "source": "test",
            "target": "test",
            "pattern": "Direct",
            "namespace": "test",
            "logical_time": 0,
            "priority": 128,
            "payload": {}
        });

        let env: EnvelopeV2 = serde_json::from_value(json_with_schema).expect("with schema");
        assert_eq!(env.schema.0, "aletheon.turn.request/v1");
    }

    #[test]
    fn unsupported_schema_is_structured_error() {
        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.unknown/v99"),
            Target::from("a"),
            Target::from("b"),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!({}),
        );
        let err = env.validate_known_schema().unwrap_err();
        assert_eq!(err.schema.0, "aletheon.unknown/v99");
        assert!(err
            .supported
            .contains(&SchemaId::TURN_REQUEST_V1.to_string()));
    }

    #[test]
    fn deadline_expiry() {
        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.process.signal/v1"),
            Target::from("kernel"),
            Target::from("agent-1"),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!(null),
        )
        .with_deadline(MonoDeadlineMillis(100));

        assert!(!env.is_expired_at(0));
        assert!(!env.is_expired_at(99));
        assert!(env.is_expired_at(100));
        assert!(env.is_expired_at(101));
    }

    #[test]
    fn no_deadline_never_expires() {
        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.process.signal/v1"),
            Target::from("kernel"),
            Target::from("agent-1"),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!(null),
        );

        assert!(!env.is_expired_at(0));
        assert!(!env.is_expired_at(u64::MAX));
    }
}
