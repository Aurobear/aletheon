//! Bounded normalization boundary for provider progress.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use fabric::{OperationId, SkillProgress};
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
