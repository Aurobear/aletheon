//! Bounded stream primitives for token/log/telemetry delivery.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    BlockProducer,
    DropOldest,
    DropNewest,
    FailStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEndReason {
    Completed,
    Cancelled,
    Failed,
    Overflow,
    ReceiverClosed,
}

#[derive(Debug, Clone)]
pub struct StreamSpec {
    pub capacity: usize,
    pub overflow: OverflowPolicy,
    pub cancel: CancellationToken,
}

impl StreamSpec {
    pub fn llm_tokens(capacity: usize) -> Self {
        Self {
            capacity,
            overflow: OverflowPolicy::BlockProducer,
            cancel: CancellationToken::new(),
        }
    }

    pub fn telemetry(capacity: usize) -> Self {
        Self {
            capacity,
            overflow: OverflowPolicy::DropOldest,
            cancel: CancellationToken::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSendError {
    Cancelled,
    Overflow,
    ReceiverClosed,
}

pub struct BoundedStream<T> {
    spec: StreamSpec,
    tx: mpsc::Sender<T>,
    rx: mpsc::Receiver<T>,
}

impl<T: Send + 'static> BoundedStream<T> {
    pub fn new(spec: StreamSpec) -> Self {
        let (tx, rx) = mpsc::channel(spec.capacity);
        Self { spec, tx, rx }
    }

    pub fn spec(&self) -> &StreamSpec {
        &self.spec
    }

    pub async fn send(&mut self, item: T) -> Result<(), StreamSendError> {
        if self.spec.cancel.is_cancelled() {
            return Err(StreamSendError::Cancelled);
        }
        match self.spec.overflow {
            OverflowPolicy::BlockProducer => self
                .tx
                .send(item)
                .await
                .map_err(|_| StreamSendError::ReceiverClosed),
            OverflowPolicy::DropNewest => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
            OverflowPolicy::FailStream => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => Err(StreamSendError::Overflow),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
            OverflowPolicy::DropOldest => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(item)) => {
                    let _ = self.rx.try_recv();
                    self.tx.try_send(item).map_err(|e| match e {
                        mpsc::error::TrySendError::Full(_) => StreamSendError::Overflow,
                        mpsc::error::TrySendError::Closed(_) => StreamSendError::ReceiverClosed,
                    })
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
        }
    }

    pub async fn recv(&mut self) -> Option<T> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Clone the sender half, usable for feeding this stream from another task.
    pub fn sender(&self) -> mpsc::Sender<T> {
        self.tx.clone()
    }
}

// ---------------------------------------------------------------------------
// Turn event stream — EnvelopeV2-based typed event channel
// ---------------------------------------------------------------------------

/// Simplified configuration for creating a `TurnEventStream`.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub capacity: usize,
    pub overflow: OverflowPolicy,
}

impl StreamConfig {
    pub fn turn_events(capacity: usize) -> Self {
        Self {
            capacity,
            overflow: OverflowPolicy::BlockProducer,
        }
    }
}

/// Well-known schema identifier for turn events.
pub const TURN_EVENT_SCHEMA: &str = "aletheon.turn.event/v1";

/// Structured error returned when a received envelope does not match the
/// expected schema or the payload cannot be deserialized.
#[derive(Debug, Clone)]
pub struct SchemaRejection {
    pub expected: String,
    pub actual: String,
    pub payload_preview: String,
}

impl std::fmt::Display for SchemaRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "schema rejection: expected '{}', got '{}' (preview: {})",
            self.expected, self.actual, self.payload_preview
        )
    }
}

impl std::error::Error for SchemaRejection {}

/// Typed event payload for turn-level streaming.
///
/// Mirrors the `cognit::harness::event_sink::Event` variants that are relevant
/// for turn orchestration and client forwarding. All variants carry enough
/// structured data to reconstruct `ClientEvent` for TUI delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEventV1 {
    // -- Turn lifecycle --
    TurnStarted {
        iteration: usize,
    },
    TurnDone {
        result: Option<String>,
    },

    // -- Streaming text --
    TextDelta {
        delta: String,
    },
    TextDeltaStop,

    // -- Tool calls --
    ToolCallStart {
        name: String,
        call_id: String,
    },
    ToolCallComplete {
        call_id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        name: String,
        call_id: String,
        content: String,
        is_error: bool,
        execution_time_ms: u64,
    },
    /// Streaming tool progress (G2). Does not enter the model context by
    /// default; clients may display it. Zero-to-many precede the single
    /// authoritative `ToolResult` terminal for a given `call_id`.
    ToolProgress {
        name: String,
        call_id: String,
        /// "text" | "structured" | "resource_ref"
        kind: String,
        /// text chunk, structured JSON, or resource uri.
        payload: serde_json::Value,
    },

    // -- Bookkeeping --
    Usage {
        tokens_in: u32,
        tokens_out: u32,
        cache_hit_tokens: u32,
        cache_miss_tokens: u32,
    },
    ContextUpdate {
        used_tokens: u32,
        max_tokens: u32,
    },
    GoalSet {
        goal: String,
        sub_goals: Vec<String>,
    },
    ModelSwitch {
        model_name: String,
    },

    // -- Awareness / collaboration --
    Approval {
        id: String,
        tool: String,
        args: serde_json::Value,
        reason: String,
    },
    AwarenessChanged {
        level: String,
        context: String,
    },
    PlanUpdate {
        version: u32,
        plan: String,
        critique: Option<String>,
        ready_for_approval: bool,
    },
    SubAgentStatusChanged {
        agent_id: String,
        status: String,
        task: String,
    },
    ModeChanged {
        mode: String,
    },

    // -- Limits / interruptions --
    Error {
        message: String,
    },
    Interrupted {
        reason: String,
    },
    BudgetExceeded {
        used: usize,
        max: usize,
    },
    CircuitBreakerTripped {
        reason: String,
    },
    CompactionTriggered {
        used_tokens: usize,
        threshold: usize,
        reason: String,
    },
    Reflection {
        summary: String,
        recommendation: String,
    },

    /// Catch-all for internal-only events not directly relevant to turn orchestration.
    Generic {
        payload: serde_json::Value,
    },
}

/// Sender half of a `TurnEventStream`.
///
/// Serializes `TurnEventV1` into an `EnvelopeV2` and pushes it into the
/// underlying mpsc channel.
pub struct TurnEventSender {
    tx: mpsc::Sender<crate::ipc::envelope_v2::EnvelopeV2>,
}

impl TurnEventSender {
    /// Serialize and send a turn event.
    pub fn send(&self, event: &TurnEventV1) -> Result<(), StreamSendError> {
        let payload = serde_json::to_value(event).map_err(|_| StreamSendError::ReceiverClosed)?;
        let envelope = crate::ipc::envelope_v2::EnvelopeV2::new(
            crate::ipc::envelope_v2::SchemaId::from(TURN_EVENT_SCHEMA),
            crate::ipc::envelope_v2::Target::from("executive"),
            crate::ipc::envelope_v2::Target::from("turn-service"),
            crate::ipc::envelope_v2::DeliveryPattern::Direct,
            crate::types::process::NamespaceId("turn-events".into()),
            payload,
        );
        self.tx.try_send(envelope).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => StreamSendError::Overflow,
            mpsc::error::TrySendError::Closed(_) => StreamSendError::ReceiverClosed,
        })
    }
}

/// Receiver half of a `TurnEventStream`.
///
/// Wraps a `BoundedStream<EnvelopeV2>` and performs schema validation and
/// deserialization on every received envelope.
pub struct TurnEventStream {
    inner: BoundedStream<crate::ipc::envelope_v2::EnvelopeV2>,
}

impl TurnEventStream {
    /// Create a new `TurnEventStream` and its paired `TurnEventSender`.
    pub fn new(config: StreamConfig) -> (Self, TurnEventSender) {
        let spec = StreamSpec {
            capacity: config.capacity,
            overflow: config.overflow,
            cancel: CancellationToken::new(),
        };
        let inner = BoundedStream::<crate::ipc::envelope_v2::EnvelopeV2>::new(spec);
        let tx = inner.sender();
        (Self { inner }, TurnEventSender { tx })
    }

    /// Receive the next event, validating the schema and deserializing the payload.
    pub async fn recv(&mut self) -> Result<TurnEventV1, SchemaRejection> {
        let envelope = self.inner.recv().await.ok_or_else(|| SchemaRejection {
            expected: TURN_EVENT_SCHEMA.to_string(),
            actual: "stream closed".to_string(),
            payload_preview: String::new(),
        })?;

        Self::decode_envelope(envelope)
    }

    /// Non-blocking receive of the next event.
    pub fn try_recv(&mut self) -> Option<Result<TurnEventV1, SchemaRejection>> {
        self.inner.try_recv().map(Self::decode_envelope)
    }

    fn decode_envelope(
        envelope: crate::ipc::envelope_v2::EnvelopeV2,
    ) -> Result<TurnEventV1, SchemaRejection> {
        if envelope.schema.0 != TURN_EVENT_SCHEMA {
            return Err(SchemaRejection {
                expected: TURN_EVENT_SCHEMA.to_string(),
                actual: envelope.schema.0.clone(),
                payload_preview: format!("{:?}", envelope.payload)
                    .chars()
                    .take(200)
                    .collect(),
            });
        }
        serde_json::from_value::<TurnEventV1>(envelope.payload).map_err(|e| SchemaRejection {
            expected: TURN_EVENT_SCHEMA.to_string(),
            actual: format!("deserialization error: {}", e),
            payload_preview: String::new(),
        })
    }
}
