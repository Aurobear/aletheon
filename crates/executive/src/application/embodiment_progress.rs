//! Bounded normalization boundary for provider progress.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use fabric::types::embodiment::SkillProgress;
use fabric::{
    EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity, EventPayload,
    EventTreeId, EventVisibility, MessageId, NamespaceId, OperationId, SchemaId, UnsequencedEvent,
};
use hardware::SkillProgressSink;

#[async_trait]
pub trait EmbodimentProgressPort: Send + Sync {
    async fn record(&self, progress: SkillProgress);
}

pub struct BoundedProgressSink {
    operation_id: OperationId,
    downstream: Arc<dyn EmbodimentProgressPort>,
    limit: usize,
    accepted: AtomicUsize,
}

impl BoundedProgressSink {
    pub fn new(
        operation_id: OperationId,
        downstream: Arc<dyn EmbodimentProgressPort>,
        limit: usize,
    ) -> Self {
        Self {
            operation_id,
            downstream,
            limit,
            accepted: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SkillProgressSink for BoundedProgressSink {
    async fn progress(&self, mut update: SkillProgress) {
        let index = self.accepted.fetch_add(1, Ordering::Relaxed);
        if index >= self.limit {
            return;
        }
        update.operation_id = self.operation_id;
        update.fraction = update.fraction.clamp(0.0, 1.0);
        self.downstream.record(update).await;
    }
}

#[derive(Default)]
pub struct RecordingEmbodimentProgress {
    updates: tokio::sync::Mutex<Vec<SkillProgress>>,
}

impl RecordingEmbodimentProgress {
    pub async fn updates(&self) -> Vec<SkillProgress> {
        self.updates.lock().await.clone()
    }
}

#[async_trait]
impl EmbodimentProgressPort for RecordingEmbodimentProgress {
    async fn record(&self, progress: SkillProgress) {
        self.updates.lock().await.push(progress);
    }
}

pub struct NoopEmbodimentProgress;

#[async_trait]
impl EmbodimentProgressPort for NoopEmbodimentProgress {
    async fn record(&self, _progress: SkillProgress) {}
}

/// Canonical progress projection: forwards bounded `SkillProgress` to a
/// `fabric::TurnEventSink` as a typed `TurnEvent::EmbodimentProgress`, carrying
/// the real operation id injected by `BoundedProgressSink`.
pub struct EventEmbodimentProgress {
    sink: Arc<dyn fabric::TurnEventSink>,
}

impl EventEmbodimentProgress {
    pub fn new(sink: Arc<dyn fabric::TurnEventSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl EmbodimentProgressPort for EventEmbodimentProgress {
    async fn record(&self, progress: SkillProgress) {
        self.sink
            .emit(fabric::TurnEvent::EmbodimentProgress {
                operation_id: progress.operation_id,
                skill: progress.skill.0.clone(),
                fraction: progress.fraction,
                note: progress.note,
            })
            .await;
    }
}

/// Minimal production sink that surfaces turn events into daemon logs. The full
/// session/UI event projection replaces this in the composition tail.
pub struct TracingTurnEventSink;

#[async_trait]
impl fabric::TurnEventSink for TracingTurnEventSink {
    async fn emit(&self, event: fabric::TurnEvent) {
        match &event {
            fabric::TurnEvent::EmbodimentProgress {
                operation_id,
                skill,
                fraction,
                note,
            } => {
                tracing::info!(
                    operation_id = %operation_id.0,
                    skill = %skill,
                    fraction = %fraction,
                    note = %note,
                    "embodiment skill progress"
                );
            }
            _ => {
                tracing::debug!(?event, "turn event");
            }
        }
    }
}

/// Session-spine projection: appends `EmbodimentProgress` to the canonical
/// `fabric::EventSpine` (rooted at the daemon session tree) so session/UI
/// consumers observe skill progress with the real operation id. Logging is
/// preserved so the daemon log keeps the existing progress line.
pub struct SpineTurnEventSink {
    spine: Arc<dyn fabric::EventSpine>,
    session_id: String,
}

impl SpineTurnEventSink {
    pub fn new(spine: Arc<dyn fabric::EventSpine>, session_id: impl Into<String>) -> Self {
        Self {
            spine,
            session_id: session_id.into(),
        }
    }
}

#[async_trait]
impl fabric::TurnEventSink for SpineTurnEventSink {
    async fn emit(&self, event: fabric::TurnEvent) {
        let fabric::TurnEvent::EmbodimentProgress {
            operation_id,
            skill,
            fraction,
            note,
        } = event
        else {
            return;
        };
        tracing::info!(
            operation_id = %operation_id.0,
            skill = %skill,
            fraction = %fraction,
            note = %note,
            "embodiment skill progress"
        );
        let session_id = self.session_id.clone();
        let event_id = EventId::new();
        let payload = serde_json::json!({
            "kind": "embodiment.skill.progress",
            "operation_id": operation_id.0,
            "skill": skill,
            "fraction": fraction,
            "note": note,
        });
        let mut envelope = EnvelopeV2::new(
            SchemaId::from("aletheon.event.embodiment_progress/v1"),
            EnvelopeV2Target("executive:embodiment-progress".into()),
            EnvelopeV2Target(format!("operation:{}", operation_id.0)),
            EnvelopeV2Delivery::FanOut,
            NamespaceId(format!("session:{session_id}")),
            payload.clone(),
        );
        envelope.id = MessageId(event_id.0);
        if let Err(error) = self.spine.append(UnsequencedEvent {
            tree_id: EventTreeId::for_root_session(&session_id),
            event_id,
            parent: None,
            identity: EventIdentity {
                root_session_id: session_id.clone(),
                session_id,
                agent_id: None,
            },
            envelope,
            visibility: EventVisibility::Control,
            payload: EventPayload::Inline { value: payload },
        }) {
            tracing::warn!(error = %error, "failed to append embodiment progress to event spine");
        }
    }
}

/// Forwarding sink whose target is bound after construction. The robot
/// embodiment port is assembled before the canonical event spine exists in the
/// daemon bootstrap, so progress is projected through this sink and bound once
/// the spine is available — always well before any robot turn can execute. Until
/// bound, events fall through to [`TracingTurnEventSink`], preserving the
/// log-only behavior.
pub struct DeferredTurnEventSink {
    inner: tokio::sync::RwLock<Option<Arc<dyn fabric::TurnEventSink>>>,
}

impl DeferredTurnEventSink {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(None),
        }
    }

    /// Bind the production sink (e.g. a [`SpineTurnEventSink`]). Later binds
    /// replace earlier ones.
    pub async fn bind(&self, sink: Arc<dyn fabric::TurnEventSink>) {
        *self.inner.write().await = Some(sink);
    }
}

impl Default for DeferredTurnEventSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl fabric::TurnEventSink for DeferredTurnEventSink {
    async fn emit(&self, event: fabric::TurnEvent) {
        match self.inner.read().await.clone() {
            Some(sink) => sink.emit(event).await,
            None => TracingTurnEventSink.emit(event).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::events::spine::{EventPosition, SpineEvent, TreeSequence};
    use fabric::types::embodiment::SkillId;
    use fabric::{MonoTime, TurnEventSink};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<fabric::TurnEvent>>);

    /// In-memory spine that records appended events for assertions.
    #[derive(Default)]
    struct RecordingSpine {
        events: Mutex<Vec<UnsequencedEvent>>,
    }

    impl fabric::EventSpine for RecordingSpine {
        fn append(&self, event: UnsequencedEvent) -> anyhow::Result<SpineEvent> {
            let recorded = event.clone();
            let envelope = recorded.envelope.clone();
            let payload = recorded.payload.clone();
            self.events.lock().unwrap().push(recorded);
            Ok(SpineEvent {
                position: EventPosition {
                    tree_id: event.tree_id,
                    event_id: event.event_id,
                    parent: event.parent,
                    sequence: TreeSequence(1),
                },
                identity: event.identity,
                schema: envelope.schema.clone(),
                visibility: event.visibility,
                envelope,
                payload,
            })
        }
    }

    #[async_trait]
    impl fabric::TurnEventSink for RecordingSink {
        async fn emit(&self, event: fabric::TurnEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[tokio::test]
    async fn event_sink_forwards_skill_progress_with_real_operation_id() {
        let sink = Arc::new(RecordingSink::default());
        let progress = EventEmbodimentProgress::new(sink.clone());

        let operation_id = fabric::OperationId::new();
        progress
            .record(SkillProgress {
                operation_id,
                skill: SkillId("kuavo.stance".into()),
                fraction: 0.5,
                note: "executing".into(),
                at: MonoTime(10),
            })
            .await;

        let recorded = sink.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        match &recorded[0] {
            fabric::TurnEvent::EmbodimentProgress {
                operation_id: op,
                skill,
                fraction,
                note,
            } => {
                assert_eq!(op, &operation_id, "must carry the injected operation id");
                assert_eq!(skill, "kuavo.stance");
                assert_eq!(*fraction, 0.5);
                assert_eq!(note, "executing");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn spine_sink_appends_progress_to_session_tree() {
        let spine = Arc::new(RecordingSpine::default());
        let sink = SpineTurnEventSink::new(spine.clone(), "session-1");

        let operation_id = fabric::OperationId::new();
        sink.emit(fabric::TurnEvent::EmbodimentProgress {
            operation_id,
            skill: "kuavo.stance".into(),
            fraction: 0.5,
            note: "executing".into(),
        })
        .await;

        let recorded = spine.events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        let event = &recorded[0];
        assert_eq!(
            event.tree_id,
            fabric::EventTreeId::for_root_session("session-1"),
            "must root at the session tree"
        );
        let payload = event.envelope.payload.clone();
        assert_eq!(payload["kind"], "embodiment.skill.progress");
        assert_eq!(payload["operation_id"], operation_id.0.to_string());
        assert_eq!(payload["skill"], "kuavo.stance");
        assert_eq!(payload["fraction"], 0.5);
    }

    #[tokio::test]
    async fn deferred_sink_logs_before_bind_and_forwards_after() {
        let spine = Arc::new(RecordingSpine::default());
        let bound = Arc::new(SpineTurnEventSink::new(spine.clone(), "session-1"));
        let deferred = Arc::new(DeferredTurnEventSink::new());

        let operation_id = fabric::OperationId::new();
        let event = fabric::TurnEvent::EmbodimentProgress {
            operation_id,
            skill: "kuavo.stance".into(),
            fraction: 0.25,
            note: "starting".into(),
        };

        // Pre-bind: falls through to the tracing sink — nothing reaches the spine.
        deferred.emit(event.clone()).await;
        assert!(spine.events.lock().unwrap().is_empty());

        // Post-bind: forwarded to the production projection.
        deferred.bind(bound).await;
        deferred.emit(event.clone()).await;
        assert_eq!(spine.events.lock().unwrap().len(), 1);
    }
}
