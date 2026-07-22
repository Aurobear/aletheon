//! Local-first composition of core Mnemosyne with optional supplemental memory.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::backends::supplemental::{
    EnqueueOutcome, SupplementalMemoryBackend, SupplementalMemoryError, SupplementalErrorCategory,
    SupplementalMemoryTransport, SupplementalRecall,
};
use crate::service::{
    ExperienceEvent, ForgetPolicy, ForgetReceipt, MemoryScope, MemoryService, RecallRequest,
    RecallSet,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeMemoryHealth {
    pub supplemental_enabled: bool,
    pub degraded: bool,
    pub error_category: Option<SupplementalErrorCategory>,
    pub queue_depth: usize,
}

#[async_trait]
pub trait SupplementalMemoryService: Send + Sync {
    fn queue_depth(&self) -> usize;

    fn metrics(&self) -> Option<crate::MemoryMetrics> {
        None
    }

    fn record(
        &self,
        event: &ExperienceEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, SupplementalMemoryError>;
    async fn recall(
        &self,
        request: RecallRequest,
        cancel: &CancellationToken,
    ) -> SupplementalRecall;
    fn forget(&self, policy: ForgetPolicy) -> Result<(), SupplementalMemoryError>;
}

#[async_trait]
impl<T: SupplementalMemoryTransport + 'static> SupplementalMemoryService for SupplementalMemoryBackend<T> {
    fn queue_depth(&self) -> usize {
        self.spool().queue_depth().unwrap_or_default()
    }

    fn metrics(&self) -> Option<crate::MemoryMetrics> {
        Some(SupplementalMemoryBackend::metrics(self).clone())
    }

    fn record(
        &self,
        event: &ExperienceEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, SupplementalMemoryError> {
        SupplementalMemoryBackend::record(self, event, now_ms)
    }

    async fn recall(
        &self,
        request: RecallRequest,
        cancel: &CancellationToken,
    ) -> SupplementalRecall {
        SupplementalMemoryBackend::recall(self, request, cancel).await
    }

    fn forget(&self, policy: ForgetPolicy) -> Result<(), SupplementalMemoryError> {
        SupplementalMemoryBackend::forget(self, policy)
    }
}

pub struct CompositeMemoryService {
    local: Arc<dyn MemoryService>,
    supplemental: Option<Arc<dyn SupplementalMemoryService>>,
    clock: Arc<dyn fabric::Clock>,
    local_budget: Duration,
    supplemental_budget: Duration,
    health: Arc<Mutex<CompositeMemoryHealth>>,
}

impl CompositeMemoryService {
    pub fn local_only(local: Arc<dyn MemoryService>, clock: Arc<dyn fabric::Clock>) -> Self {
        Self::new(
            local,
            None,
            clock,
            Duration::from_millis(500),
            Duration::from_millis(250),
        )
    }

    pub fn new(
        local: Arc<dyn MemoryService>,
        supplemental: Option<Arc<dyn SupplementalMemoryService>>,
        clock: Arc<dyn fabric::Clock>,
        local_budget: Duration,
        supplemental_budget: Duration,
    ) -> Self {
        let enabled = supplemental.is_some();
        Self {
            local,
            supplemental,
            clock,
            local_budget,
            supplemental_budget,
            health: Arc::new(Mutex::new(CompositeMemoryHealth {
                supplemental_enabled: enabled,
                degraded: false,
                error_category: None,
                queue_depth: 0,
            })),
        }
    }

    pub fn health_handle(&self) -> Arc<Mutex<CompositeMemoryHealth>> {
        self.health.clone()
    }

    fn selected(event: &ExperienceEvent) -> bool {
        matches!(
            event,
            ExperienceEvent::ArchitectureDecision { .. } | ExperienceEvent::GoalOutcome { .. }
        )
    }

    fn update_health(
        &self,
        degraded: bool,
        category: Option<SupplementalErrorCategory>,
        queue_depth: usize,
    ) {
        let mut health = self
            .health
            .lock()
            .expect("composite memory health mutex poisoned");
        health.degraded = degraded;
        health.error_category = category;
        health.queue_depth = queue_depth;
    }
}

#[async_trait]
impl MemoryService for CompositeMemoryService {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        self.local.record(event.clone()).await?;
        let Some(supplemental) = &self.supplemental else {
            return Ok(());
        };
        if !Self::selected(&event) {
            return Ok(());
        }
        let now_ms = self.clock.wall_now().0.max(0);
        let queue_depth = supplemental.queue_depth();
        match supplemental.record(&event, now_ms) {
            Ok(_) => {
                let new_depth = supplemental.queue_depth();
                self.health
                    .lock()
                    .expect("composite memory health mutex poisoned")
                    .queue_depth = new_depth;
            }
            Err(error) => {
                tracing::warn!(error = %error, "supplemental memory enqueue degraded");
                self.update_health(true, Some(SupplementalErrorCategory::Spool), queue_depth);
            }
        }
        Ok(())
    }

    async fn recall(&self, request: RecallRequest) -> anyhow::Result<RecallSet> {
        let local_request = request.clone();
        let local = tokio::time::timeout(self.local_budget, self.local.recall(local_request));
        let supplemental = async {
            match &self.supplemental {
                Some(service) => Some(
                    match tokio::time::timeout(
                        self.supplemental_budget,
                        service.recall(request.clone(), &CancellationToken::new()),
                    )
                    .await
                    {
                        Ok(recall) => recall,
                        Err(_) => SupplementalRecall {
                            items: Vec::new(),
                            health: crate::backends::supplemental::SupplementalRecallHealth {
                                degraded: true,
                                error_category: Some(SupplementalErrorCategory::Timeout),
                                queue_depth: service.queue_depth(),
                            },
                        },
                    },
                ),
                None => None,
            }
        };
        let (local, supplemental) = tokio::join!(local, supplemental);
        let local = local.map_err(|_| anyhow::anyhow!("local memory recall timed out"))??;
        let Some(supplemental) = supplemental else {
            return Ok(local);
        };
        self.update_health(
            supplemental.health.degraded,
            supplemental.health.error_category,
            supplemental.health.queue_depth,
        );
        let mut supplemental_items = supplemental.items;
        for item in &mut supplemental_items {
            item.scope = crate::MemoryScope::Session(request.session.clone());
        }
        let mut degraded_sources = local.degraded_sources;
        if supplemental.health.degraded {
            degraded_sources.push("supplemental_memory".into());
        }
        let metrics = self
            .supplemental
            .as_ref()
            .and_then(|service| service.metrics());
        Ok(RecallSet {
            items: crate::recall::merge_items(
                [local.items, supplemental_items],
                &request,
                metrics.as_ref(),
            ),
            degraded_sources,
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        self.local.consolidate(scope).await
    }

    async fn preview_forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        self.local.preview_forget(policy).await
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        // The local retention transaction persists remote-pending state. The
        // supplemental worker propagates that durable outbox asynchronously.
        self.local.forget(policy).await
    }

    async fn synthesize(
        &self,
        request: crate::service::SynthesisRequest,
    ) -> anyhow::Result<crate::service::SynthesisResult> {
        self.local.synthesize(request).await
    }
}
