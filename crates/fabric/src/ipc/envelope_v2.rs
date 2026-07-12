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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    // ------------------------------------------------------------------
    // Legacy converter
    // ------------------------------------------------------------------

    /// Convert a legacy [`super::envelope::Envelope`] into an `EnvelopeV2`.
    ///
    /// This is a **best-effort lossy mapping**:
    ///
    /// | Legacy field        | V2 field           | Mapping |
    /// |---------------------|--------------------|---------|
    /// | `id` (u64)          | `id` (MessageId)   | fresh UUID v4 — legacy counter is not preserved |
    /// | (none)              | `schema`           | `"aletheon.legacy.envelope/v0"` |
    /// | `source` (Endpoint) | `source` (Target)  | `"{variant}:{value}"` string |
    /// | `target` (Target)   | `target` (Target)  | `"{variant}:{value}"` string |
    /// | `pattern`           | `pattern`          | best-match mapping (see below) |
    /// | `correlation_id`    | `correlation_id`   | wrapped in `MessageId` (lossy: u64 → UUID) |
    /// | `timestamp_ms`      | `logical_time`     | reused as logical time |
    /// | `ttl_ms`            | `deadline`         | `timestamp_ms + ttl_ms` → `MonoDeadlineMillis` |
    /// | `priority`          | `priority`         | Priority enum → u8 |
    /// | `payload`           | `payload`          | Json→pass through; Binary→base64-wrapped; Empty→null |
    pub fn from_legacy(legacy: &super::envelope::Envelope) -> Self {
        use super::envelope::{Pattern, Payload};

        let source = endpoint_to_target_str(&legacy.source);
        let target = legacy_target_to_target_str(&legacy.target);

        let pattern = match legacy.pattern {
            Pattern::Request { .. } => DeliveryPattern::RequestResponse,
            Pattern::Response => DeliveryPattern::Direct,
            Pattern::Publish => DeliveryPattern::FanOut,
            Pattern::FireAndForget => DeliveryPattern::Direct,
            Pattern::Stream { .. } => DeliveryPattern::Direct,
        };

        let correlation_id = legacy
            .correlation_id
            .map(|cid| MessageId(uuid_from_u64(cid)));

        let payload = match &legacy.payload {
            Payload::Json(v) => v.clone(),
            Payload::Binary(b) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b);
                serde_json::Value::String(encoded)
            }
            Payload::Empty => serde_json::Value::Null,
        };

        let priority = legacy.priority.into_u8();

        let deadline = legacy
            .ttl_ms
            .map(|ttl| MonoDeadlineMillis(legacy.timestamp_ms.saturating_add(ttl)));

        Self {
            id: MessageId::new(),
            schema: SchemaId("aletheon.legacy.envelope/v0".into()),
            source,
            target,
            pattern,
            operation_id: None,
            causation_id: None,
            correlation_id,
            namespace: NamespaceId("legacy".into()),
            logical_time: legacy.timestamp_ms,
            deadline,
            priority,
            payload,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn endpoint_to_target_str(e: &super::envelope::Endpoint) -> Target {
    use super::envelope::Endpoint;
    match e {
        Endpoint::Module(m) => Target(format!("module:{m:?}")),
        Endpoint::Agent(pid) => Target(format!("agent:{pid}")),
        Endpoint::System => Target("system".into()),
    }
}

fn legacy_target_to_target_str(t: &super::envelope::Target) -> Target {
    use super::envelope::Target as LegacyTarget;
    match t {
        LegacyTarget::Module(m) => Target(format!("module:{m:?}")),
        LegacyTarget::Agent(pid) => Target(format!("agent:{pid}")),
        LegacyTarget::Topic(name) => Target(format!("topic:{name}")),
        LegacyTarget::Broadcast => Target("broadcast".into()),
    }
}

/// Deterministic UUID v5 from a u64 (namespace = DNS "aletheon.local").
fn uuid_from_u64(n: u64) -> Uuid {
    let ns = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8") // DNS namespace
        .unwrap_or(Uuid::nil());
    Uuid::new_v5(&ns, n.to_le_bytes().as_ref())
}

// ---------------------------------------------------------------------------
// Priority conversion (compatibility with legacy Priority enum)
// ---------------------------------------------------------------------------

/// Extend the legacy `Priority` enum with a `u8` conversion so the
/// `From<Priority>` impl in `EnvelopeV2::from_legacy` works without
/// modifying the legacy event module.
trait PriorityU8 {
    fn into_u8(self) -> u8;
}

impl PriorityU8 for crate::event::Priority {
    fn into_u8(self) -> u8 {
        match self {
            crate::event::Priority::Low => 50,
            crate::event::Priority::Normal => 128,
            crate::event::Priority::High => 200,
            crate::event::Priority::Critical => 255,
            crate::event::Priority::Background => 10,
        }
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
    pub const LEGACY_ENVELOPE_V0: &'static str = "aletheon.legacy.envelope/v0";

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

    /// Map a legacy `EventType` to its corresponding `SchemaId` string.
    ///
    /// This is the migration bridge: old code that used `EventType` routing
    /// can use this to produce an `EnvelopeV2` with the correct schema.
    pub fn from_event_type(et: &crate::event::EventType) -> &'static str {
        use crate::event::EventType;
        match et {
            EventType::UserIntent => Self::EVENT_USER_INTENT_V1,
            EventType::UserFeedback => Self::EVENT_USER_FEEDBACK_V1,
            EventType::EnvironmentChange => Self::EVENT_ENVIRONMENT_CHANGE_V1,
            EventType::PerceptionUpdate => Self::EVENT_PERCEPTION_UPDATE_V1,
            EventType::ToolObservation => Self::EVENT_TOOL_OBSERVATION_V1,
            EventType::ToolError => Self::EVENT_TOOL_ERROR_V1,
            EventType::ActionCompleted => Self::EVENT_ACTION_COMPLETED_V1,
            EventType::MemoryStored => Self::EVENT_MEMORY_STORED_V1,
            EventType::MemoryRecalled => Self::EVENT_MEMORY_RECALLED_V1,
            EventType::MemoryCompacted => Self::EVENT_MEMORY_COMPACTED_V1,
            EventType::IdentityQuery => Self::EVENT_IDENTITY_QUERY_V1,
            EventType::BoundaryCheck => Self::EVENT_BOUNDARY_CHECK_V1,
            EventType::ConflictDetected => Self::EVENT_CONFLICT_DETECTED_V1,
            EventType::RejectionIssued => Self::EVENT_REJECTION_ISSUED_V1,
            EventType::PlanGenerated => Self::EVENT_PLAN_GENERATED_V1,
            EventType::ReflectionComplete => Self::EVENT_REFLECTION_COMPLETE_V1,
            EventType::CriticismRaised => Self::EVENT_CRITICISM_RAISED_V1,
            EventType::MutationIntent => Self::EVENT_MUTATION_INTENT_V1,
            EventType::RuntimeCandidate => Self::EVENT_RUNTIME_CANDIDATE_V1,
            EventType::MigrationStarted => Self::EVENT_MIGRATION_STARTED_V1,
            EventType::MigrationComplete => Self::EVENT_MIGRATION_COMPLETE_V1,
            EventType::SubsystemStarted => Self::EVENT_SUBSYSTEM_STARTED_V1,
            EventType::SubsystemFailed => Self::EVENT_SUBSYSTEM_FAILED_V1,
            EventType::HealthCheck => Self::EVENT_HEALTH_CHECK_V1,
            EventType::AgentStarted => Self::EVENT_AGENT_STARTED_V1,
            EventType::AgentStopped => Self::EVENT_AGENT_STOPPED_V1,
            EventType::AgentFailed => Self::EVENT_AGENT_FAILED_V1,
            EventType::ScheduledTaskFired => Self::EVENT_SCHEDULED_TASK_FIRED_V1,
            EventType::BootPhaseChanged => Self::EVENT_BOOT_PHASE_CHANGED_V1,
            EventType::ReActIterationStart => Self::EVENT_REACT_ITERATION_START_V1,
            EventType::ReActIterationEnd => Self::EVENT_REACT_ITERATION_END_V1,
            EventType::AgentForkCompleted => Self::EVENT_AGENT_FORK_COMPLETED_V1,
            EventType::RuleExtracted => Self::EVENT_RULE_EXTRACTED_V1,
            EventType::EvolutionTriggered => Self::EVENT_EVOLUTION_TRIGGERED_V1,
            EventType::EvolutionResult => Self::EVENT_EVOLUTION_RESULT_V1,
            EventType::CognitivePulse => Self::EVENT_COGNITIVE_PULSE_V1,
            EventType::AgentSpawned => Self::EVENT_AGENT_SPAWNED_V1,
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

    // -- SchemaId::from_event_type migration bridge tests -----------------

    #[test]
    fn from_event_type_covers_all_variants_without_panic() {
        use crate::event::EventType;
        // Every variant must map to a distinct non-empty schema string.
        let variants = [
            EventType::UserIntent,
            EventType::UserFeedback,
            EventType::EnvironmentChange,
            EventType::PerceptionUpdate,
            EventType::ToolObservation,
            EventType::ToolError,
            EventType::ActionCompleted,
            EventType::MemoryStored,
            EventType::MemoryRecalled,
            EventType::MemoryCompacted,
            EventType::IdentityQuery,
            EventType::BoundaryCheck,
            EventType::ConflictDetected,
            EventType::RejectionIssued,
            EventType::PlanGenerated,
            EventType::ReflectionComplete,
            EventType::CriticismRaised,
            EventType::MutationIntent,
            EventType::RuntimeCandidate,
            EventType::MigrationStarted,
            EventType::MigrationComplete,
            EventType::SubsystemStarted,
            EventType::SubsystemFailed,
            EventType::HealthCheck,
            EventType::AgentStarted,
            EventType::AgentStopped,
            EventType::AgentFailed,
            EventType::ScheduledTaskFired,
            EventType::BootPhaseChanged,
            EventType::ReActIterationStart,
            EventType::ReActIterationEnd,
            EventType::AgentForkCompleted,
            EventType::RuleExtracted,
            EventType::EvolutionTriggered,
            EventType::EvolutionResult,
            EventType::CognitivePulse,
            EventType::AgentSpawned,
        ];
        let mut seen = std::collections::HashSet::new();
        for v in &variants {
            let schema = SchemaId::from_event_type(v);
            assert!(!schema.is_empty(), "empty schema for {v:?}");
            assert!(seen.insert(schema), "duplicate schema '{schema}' for {v:?}");
        }
        assert_eq!(seen.len(), variants.len(), "schema count mismatch");
    }

    #[test]
    fn from_event_type_schemas_are_valid() {
        use crate::event::EventType;
        // Spot-check a few well-known mappings.
        assert_eq!(
            SchemaId::from_event_type(&EventType::UserIntent),
            "aletheon.event.user_intent/v1"
        );
        assert_eq!(
            SchemaId::from_event_type(&EventType::ToolObservation),
            "aletheon.event.tool_observation/v1"
        );
        assert_eq!(
            SchemaId::from_event_type(&EventType::AgentStarted),
            "aletheon.event.agent_started/v1"
        );
    }
}
