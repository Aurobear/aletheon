//! Bounded normalization boundary for provider progress.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use fabric::types::embodiment::SkillProgress;
use fabric::OperationId;
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

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::SkillId;
    use fabric::MonoTime;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<fabric::TurnEvent>>);

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
                operation_id: operation_id.clone(),
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
}
