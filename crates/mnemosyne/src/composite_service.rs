//! Local-first composition of core Mnemosyne with optional supplemental memory.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::backends::gbrain::{
    EnqueueOutcome, GbrainBackend, GbrainBackendError, SupplementalErrorCategory,
    SupplementalMemoryTransport, SupplementalRecall,
};
use crate::service::{
    ExperienceEvent, ForgetPolicy, MemoryScope, MemoryService, RecallItem, RecallRequest,
    RecallSet, TemporalState,
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

    fn record(
        &self,
        event: &ExperienceEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, GbrainBackendError>;
    async fn recall(
        &self,
        request: RecallRequest,
        cancel: &CancellationToken,
    ) -> SupplementalRecall;
    fn forget(&self, policy: ForgetPolicy) -> Result<(), GbrainBackendError>;
}

#[async_trait]
impl<T: SupplementalMemoryTransport + 'static> SupplementalMemoryService for GbrainBackend<T> {
    fn queue_depth(&self) -> usize {
        self.spool().queue_depth().unwrap_or_default()
    }

    fn record(
        &self,
        event: &ExperienceEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, GbrainBackendError> {
        GbrainBackend::record(self, event, now_ms)
    }

    async fn recall(
        &self,
        request: RecallRequest,
        cancel: &CancellationToken,
    ) -> SupplementalRecall {
        GbrainBackend::recall(self, request, cancel).await
    }

    fn forget(&self, policy: ForgetPolicy) -> Result<(), GbrainBackendError> {
        GbrainBackend::forget(self, policy)
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
                            health: crate::backends::gbrain::SupplementalRecallHealth {
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
        Ok(RecallSet {
            items: merge_items(local.items, supplemental.items, &request),
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        self.local.consolidate(scope).await
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<()> {
        self.local.forget(policy.clone()).await?;
        if let Some(supplemental) = &self.supplemental {
            supplemental.forget(policy)?;
        }
        Ok(())
    }
}

fn merge_items(
    local: Vec<RecallItem>,
    remote: Vec<RecallItem>,
    request: &RecallRequest,
) -> Vec<RecallItem> {
    let mut superseded = HashSet::new();
    for item in local.iter().chain(&remote) {
        if let Some(previous) = &item.metadata.supersedes {
            superseded.insert(previous.clone());
        }
    }
    let mut by_key: HashMap<(String, String), RecallItem> = HashMap::new();
    for mut item in local.into_iter().chain(remote) {
        if superseded.contains(&item.metadata.record_id) {
            item.temporal_state = TemporalState::Superseded;
        }
        let key = (
            item.metadata.provenance.source.clone(),
            if request.include_historical {
                format!(
                    "{}#{}",
                    item.metadata.provenance.source_id, item.metadata.record_id
                )
            } else {
                item.metadata.provenance.source_id.clone()
            },
        );
        match by_key.get(&key) {
            Some(existing) if !prefer(&item, existing) => {}
            _ => {
                by_key.insert(key, item);
            }
        }
    }
    let mut items = by_key
        .into_values()
        .filter(|item| {
            request.include_historical
                || !matches!(
                    item.temporal_state,
                    TemporalState::Superseded | TemporalState::Expired
                )
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        state_rank(left.temporal_state)
            .cmp(&state_rank(right.temporal_state))
            .then_with(|| {
                right
                    .metadata
                    .observed_time
                    .cmp(&left.metadata.observed_time)
            })
            .then_with(|| {
                right
                    .metadata
                    .confidence
                    .total_cmp(&left.metadata.confidence)
            })
            .then_with(|| left.metadata.record_id.cmp(&right.metadata.record_id))
    });
    let mut bytes = 0usize;
    items
        .into_iter()
        .take(request.max_items)
        .take_while(|item| {
            if bytes.saturating_add(item.content.len()) > request.max_content_bytes {
                false
            } else {
                bytes += item.content.len();
                true
            }
        })
        .collect()
}

fn prefer(candidate: &RecallItem, existing: &RecallItem) -> bool {
    state_rank(candidate.temporal_state) < state_rank(existing.temporal_state)
        || (state_rank(candidate.temporal_state) == state_rank(existing.temporal_state)
            && (
                candidate
                    .metadata
                    .valid_from
                    .unwrap_or(candidate.metadata.observed_time),
                candidate.metadata.observed_time,
                candidate.metadata.confidence,
            ) > (
                existing
                    .metadata
                    .valid_from
                    .unwrap_or(existing.metadata.observed_time),
                existing.metadata.observed_time,
                existing.metadata.confidence,
            ))
}

fn state_rank(state: TemporalState) -> u8 {
    match state {
        TemporalState::Current => 0,
        TemporalState::Unknown => 1,
        TemporalState::Superseded => 2,
        TemporalState::Expired => 3,
    }
}
