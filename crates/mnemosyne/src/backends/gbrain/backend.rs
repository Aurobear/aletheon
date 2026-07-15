//! Transport-neutral supplemental-memory backend backed by the SQLite spool.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::config::GbrainBackendConfig;
use super::page::GbrainPage;
use super::spool::{EnqueueOutcome, GbrainSpool, SpoolError};
use crate::service::{
    ExperienceEvent, ForgetPolicy, MemorySensitivity, RecallItem, RecallRequest, TemporalState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplementalErrorCategory {
    Auth,
    Schema,
    InvalidPage,
    RejectedArguments,
    Timeout,
    Cancelled,
    RateLimited,
    Provider,
    Transport,
    MalformedResponse,
    OversizedResponse,
    Spool,
    Unsupported,
}

impl SupplementalErrorCategory {
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Cancelled | Self::RateLimited | Self::Provider | Self::Transport
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("supplemental memory {category:?}: {message}")]
pub struct SupplementalTransportError {
    pub category: SupplementalErrorCategory,
    pub message: &'static str,
}

impl SupplementalTransportError {
    pub fn new(category: SupplementalErrorCategory, message: &'static str) -> Self {
        Self { category, message }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SupplementalHit {
    pub source_id: String,
    pub slug: String,
    pub content: String,
    pub score: f64,
}

#[async_trait]
pub trait SupplementalMemoryTransport: Send + Sync {
    fn set_queue_depth(&self, _queue_depth: usize) {}

    async fn put_page(
        &self,
        page: &GbrainPage,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, SupplementalTransportError>;
    async fn query(
        &self,
        query: &str,
        source_id: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError>;
    async fn search(
        &self,
        query: &str,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<SupplementalHit>, SupplementalTransportError>;
    async fn get_page(
        &self,
        slug: &str,
        cancel: &CancellationToken,
    ) -> Result<String, SupplementalTransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplementalRecallHealth {
    pub degraded: bool,
    pub error_category: Option<SupplementalErrorCategory>,
    pub queue_depth: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SupplementalRecall {
    pub items: Vec<RecallItem>,
    pub health: SupplementalRecallHealth,
}

#[derive(Debug, thiserror::Error)]
pub enum GbrainBackendError {
    #[error(transparent)]
    Spool(#[from] SpoolError),
    #[error("supplemental forget is unsupported")]
    Unsupported,
    #[error("supplemental memory record is invalid")]
    InvalidRecord,
}

pub struct GbrainBackend<T: SupplementalMemoryTransport> {
    spool: Arc<GbrainSpool>,
    transport: Arc<T>,
    config: GbrainBackendConfig,
}

impl<T: SupplementalMemoryTransport> GbrainBackend<T> {
    pub fn new(spool: Arc<GbrainSpool>, transport: Arc<T>, config: GbrainBackendConfig) -> Self {
        Self {
            spool,
            transport,
            config,
        }
    }

    pub fn spool(&self) -> &Arc<GbrainSpool> {
        &self.spool
    }

    /// Commits policy-selected records to SQLite; never waits for MCP delivery.
    pub fn record(
        &self,
        event: &ExperienceEvent,
        now_ms: i64,
    ) -> Result<EnqueueOutcome, GbrainBackendError> {
        if !self.config.projection_enabled {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        }
        let metadata = match event {
            ExperienceEvent::ArchitectureDecision { metadata, .. }
            | ExperienceEvent::GoalOutcome { metadata, .. }
            | ExperienceEvent::Message { metadata, .. }
            | ExperienceEvent::Reflection { metadata, .. } => metadata,
        };
        if matches!(
            metadata.sensitivity,
            MemorySensitivity::Confidential | MemorySensitivity::Restricted
        ) {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        }
        let Some(page) =
            GbrainPage::from_event(event).map_err(|_| GbrainBackendError::InvalidRecord)?
        else {
            return Ok(EnqueueOutcome::ExcludedSensitive);
        };
        let outcome = self.spool.enqueue(
            &metadata.record_id,
            &page,
            metadata.sensitivity.clone(),
            now_ms,
        )?;
        self.transport
            .set_queue_depth(self.spool.queue_depth().unwrap_or_default());
        Ok(outcome)
    }

    pub async fn recall(
        &self,
        req: RecallRequest,
        cancel: &CancellationToken,
    ) -> SupplementalRecall {
        if !self.config.enabled {
            return self.empty_health(None);
        }
        if req.validate().is_err() {
            return self.empty_health(Some(SupplementalErrorCategory::RejectedArguments));
        }
        let budget = Duration::from_millis(self.config.request_timeout_ms);
        let result = tokio::select! {
            _ = cancel.cancelled() => return self.empty_health(Some(SupplementalErrorCategory::Cancelled)),
            result = tokio::time::timeout(budget, self.recall_inner(&req, cancel)) => result,
        };
        match result {
            Err(_) => self.empty_health(Some(SupplementalErrorCategory::Timeout)),
            Ok(Ok((items, category))) => SupplementalRecall {
                items,
                health: SupplementalRecallHealth {
                    degraded: category.is_some(),
                    error_category: category,
                    queue_depth: self.spool.queue_depth().unwrap_or_default(),
                },
            },
            Ok(Err(category)) => self.empty_health(Some(category)),
        }
    }

    pub fn forget(&self, _policy: ForgetPolicy) -> Result<(), GbrainBackendError> {
        Err(GbrainBackendError::Unsupported)
    }

    async fn recall_inner(
        &self,
        req: &RecallRequest,
        cancel: &CancellationToken,
    ) -> Result<(Vec<RecallItem>, Option<SupplementalErrorCategory>), SupplementalErrorCategory>
    {
        let limit = req.max_items.min(self.config.recall_limit).max(1);
        let mut hits = Vec::new();
        let mut last_error = None;
        for source in &self.config.read_sources {
            match self
                .transport
                .query(&req.query, source, limit, cancel)
                .await
            {
                Ok(source_hits) => hits.extend(
                    source_hits
                        .into_iter()
                        .filter(|hit| hit.source_id == *source),
                ),
                Err(error) => last_error = Some(error.category),
            }
            if hits.len() >= limit {
                break;
            }
        }
        if hits.is_empty() && last_error.is_some() {
            match self.transport.search(&req.query, limit, cancel).await {
                Ok(search_hits) => hits.extend(search_hits.into_iter().filter(|hit| {
                    self.config
                        .read_sources
                        .iter()
                        .any(|source| source == &hit.source_id)
                })),
                Err(error) => return Err(error.category),
            }
        }
        if hits.is_empty() && last_error.is_some() {
            return Ok((Vec::new(), last_error));
        }

        hits.truncate(limit);
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut used_bytes = 0usize;
        for hit in hits {
            let page_content = if hit.content.starts_with("---\n") {
                hit.content
            } else {
                self.transport
                    .get_page(&hit.slug, cancel)
                    .await
                    .map_err(|error| error.category)?
            };
            let page = GbrainPage {
                slug: hit.slug,
                content: page_content,
            };
            let item = page
                .to_recall_item(req.current_at)
                .map_err(|_| SupplementalErrorCategory::MalformedResponse)?;
            if matches!(
                item.metadata.sensitivity,
                MemorySensitivity::Confidential | MemorySensitivity::Restricted
            ) {
                continue;
            }
            if !req.include_historical
                && matches!(
                    item.temporal_state,
                    TemporalState::Superseded | TemporalState::Expired
                )
            {
                continue;
            }
            if !seen.insert(item.metadata.record_id.clone()) {
                continue;
            }
            if used_bytes.saturating_add(item.content.len()) > req.max_content_bytes {
                break;
            }
            used_bytes += item.content.len();
            items.push(item);
        }
        Ok((items, last_error))
    }

    fn empty_health(&self, category: Option<SupplementalErrorCategory>) -> SupplementalRecall {
        SupplementalRecall {
            items: Vec::new(),
            health: SupplementalRecallHealth {
                degraded: category.is_some(),
                error_category: category,
                queue_depth: self.spool.queue_depth().unwrap_or_default(),
            },
        }
    }
}
