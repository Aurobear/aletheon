//! Durable Google event outbox claiming and idempotent delivery.

use super::{GoogleSyncStore, SyncStoreError};
use async_trait::async_trait;
use fabric::{ExternalEventEnvelope, ExternalEventId};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::r#impl::channel::store::ChannelStore;
use crate::r#impl::goal::GoalCoordinator;
use fabric::channel::{MessageContent, OutboundMessage};
use fabric::{ConversationId, GoogleEvent};
use std::path::Path;

#[async_trait]
pub trait GoogleEventSink: Send + Sync {
    /// Implementations must treat `idempotency_key` as a durable unique key.
    async fn deliver(
        &self,
        idempotency_key: ExternalEventId,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub claimed: usize,
    pub delivered: usize,
    pub failed: usize,
}

#[derive(Clone)]
pub struct GoogleEventDispatcher {
    store: Arc<Mutex<GoogleSyncStore>>,
    sink: Arc<dyn GoogleEventSink>,
    owner: String,
    claim_duration_ms: i64,
}

impl GoogleEventDispatcher {
    pub fn new(
        store: Arc<Mutex<GoogleSyncStore>>,
        sink: Arc<dyn GoogleEventSink>,
        owner: String,
        claim_duration_ms: i64,
    ) -> Result<Self, SyncStoreError> {
        if owner.is_empty() || owner.len() > 256 || !(1_000..=300_000).contains(&claim_duration_ms)
        {
            return Err(SyncStoreError::InvalidInput);
        }
        Ok(Self {
            store,
            sink,
            owner,
            claim_duration_ms,
        })
    }

    pub async fn dispatch_due(
        &self,
        now_ms: i64,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<DispatchOutcome, SyncStoreError> {
        let claims = self.store.lock().unwrap().claim_outbox(
            &self.owner,
            now_ms,
            self.claim_duration_ms,
            limit,
        )?;
        let mut outcome = DispatchOutcome {
            claimed: claims.len(),
            delivered: 0,
            failed: 0,
        };
        for claim in claims {
            if cancel.is_cancelled() {
                break;
            }
            match self
                .sink
                .deliver(claim.event.id, &claim.event, cancel)
                .await
            {
                Ok(()) => {
                    if self.store.lock().unwrap().acknowledge_outbox(
                        &claim.outbox_id,
                        &self.owner,
                        now_ms,
                    )? {
                        outcome.delivered += 1;
                    }
                }
                Err(code) => {
                    let code = bounded_error_code(&code);
                    self.store.lock().unwrap().fail_outbox(
                        &claim.outbox_id,
                        &self.owner,
                        &code,
                        now_ms,
                    )?;
                    outcome.failed += 1;
                }
            }
        }
        Ok(outcome)
    }
}

fn bounded_error_code(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "delivery_failed".into();
    }
    value.chars().take(256).collect()
}

pub trait GoogleCurrentTaskProjection: Send + Sync {
    fn project_current_task(
        &self,
        principal: &fabric::PrincipalId,
        task_id: &str,
        event: &ExternalEventEnvelope,
    ) -> Result<(), String>;
}

pub trait GoogleMemoryProposalSink: Send + Sync {
    fn propose_with_provenance(
        &self,
        principal: &fabric::PrincipalId,
        event: &ExternalEventEnvelope,
    ) -> Result<(), String>;
}

#[async_trait]
pub trait GoogleMailIngressSink: Send + Sync {
    async fn ingest_mail(
        &self,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<(), String>;
}

#[async_trait]
impl GoogleMailIngressSink for crate::r#impl::channel::gmail::GmailGoalEventIngress {
    async fn ingest_mail(
        &self,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.ingest(event, cancel).await.map(|_| ())
    }
}

pub trait GoogleNotificationSink: Send + Sync {
    fn enqueue(
        &self,
        conversation_id: ConversationId,
        event: &ExternalEventEnvelope,
    ) -> Result<bool, String>;
}

/// Production notification sink that persists directly into the shared channel
/// outbox. It does not need to hold the async Telegram poll-loop router.
pub struct DurableGoogleNotificationSink {
    store: Mutex<ChannelStore>,
}

impl DurableGoogleNotificationSink {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            store: Mutex::new(ChannelStore::open(path)?),
        })
    }
}

impl GoogleNotificationSink for DurableGoogleNotificationSink {
    fn enqueue(
        &self,
        conversation_id: ConversationId,
        event: &ExternalEventEnvelope,
    ) -> Result<bool, String> {
        let Some(text) = bounded_notification_text(event) else {
            return Ok(false);
        };
        self.store
            .lock()
            .unwrap()
            .enqueue_outbound(
                "telegram",
                &OutboundMessage {
                    conversation_id,
                    content: MessageContent::Text { text },
                    actions: Vec::new(),
                    reply_to: None,
                    correlation_id: event.id.to_string(),
                },
            )
            .map_err(|error| error.to_string())
    }
}

pub struct GoogleEventRouter {
    store: Arc<Mutex<GoogleSyncStore>>,
    goals: Arc<GoalCoordinator>,
    notifications: Arc<dyn GoogleNotificationSink>,
    mail_ingress: Option<Arc<dyn GoogleMailIngressSink>>,
    current_tasks: Option<Arc<dyn GoogleCurrentTaskProjection>>,
    memory_proposals: Option<Arc<dyn GoogleMemoryProposalSink>>,
}

impl GoogleEventRouter {
    pub fn new_with_notifications(
        store: Arc<Mutex<GoogleSyncStore>>,
        goals: Arc<GoalCoordinator>,
        notifications: Arc<dyn GoogleNotificationSink>,
    ) -> Self {
        Self {
            store,
            goals,
            notifications,
            mail_ingress: None,
            current_tasks: None,
            memory_proposals: None,
        }
    }

    pub fn with_current_tasks(mut self, sink: Arc<dyn GoogleCurrentTaskProjection>) -> Self {
        self.current_tasks = Some(sink);
        self
    }

    pub fn with_mail_ingress(mut self, sink: Arc<dyn GoogleMailIngressSink>) -> Self {
        self.mail_ingress = Some(sink);
        self
    }

    pub fn with_memory_proposals(mut self, sink: Arc<dyn GoogleMemoryProposalSink>) -> Self {
        self.memory_proposals = Some(sink);
        self
    }
}

#[async_trait]
impl GoogleEventSink for GoogleEventRouter {
    async fn deliver(
        &self,
        _idempotency_key: ExternalEventId,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let stream = stream_for(event);
        let subscriptions = {
            let store = self.store.lock().unwrap();
            if !store
                .account_is_active(event.account_id)
                .map_err(|error| error.to_string())?
            {
                return Ok(());
            }
            let generation = store
                .cursor(event.account_id, stream)
                .map_err(|error| error.to_string())?
                .map(|cursor| cursor.generation)
                .unwrap_or(0);
            store
                .matching_subscriptions(event, stream, generation)
                .map_err(|error| error.to_string())?
        };
        if matches!(event.event, GoogleEvent::MailReceived(_)) {
            if let Some(ingress) = &self.mail_ingress {
                ingress.ingest_mail(event, cancel).await?;
            }
        }
        for subscription in subscriptions {
            self.goals
                .wake_for_google_event(&subscription.principal_id, event)
                .map_err(|error| error.to_string())?;
            if let Some(conversation) = subscription.query.telegram_conversation_id {
                self.notifications
                    .enqueue(ConversationId(conversation), event)?;
            }
            if let (Some(task_id), Some(sink)) = (
                subscription.query.current_task_id.as_deref(),
                self.current_tasks.as_ref(),
            ) {
                sink.project_current_task(&subscription.principal_id, task_id, event)?;
            }
            if subscription.query.propose_memory {
                if let Some(sink) = &self.memory_proposals {
                    sink.propose_with_provenance(&subscription.principal_id, event)?;
                }
            }
        }
        Ok(())
    }
}

fn bounded_notification_text(event: &ExternalEventEnvelope) -> Option<String> {
    let summary = match &event.event {
        GoogleEvent::MailReceived(change) | GoogleEvent::MailUpdated(change) => format!(
            "Important mail from {}: {}",
            change.message.from, change.message.subject
        ),
        GoogleEvent::CalendarEventCreated(calendar)
        | GoogleEvent::CalendarEventUpdated(calendar) => {
            format!("Calendar changed: {}", calendar.summary)
        }
        GoogleEvent::CalendarEventDeleted(_) => "Calendar event cancelled".into(),
        _ => return None,
    };
    Some(summary.chars().take(2_000).collect())
}

fn stream_for(event: &ExternalEventEnvelope) -> super::SyncStream {
    match event.event {
        fabric::GoogleEvent::MailReceived(_)
        | fabric::GoogleEvent::MailUpdated(_)
        | fabric::GoogleEvent::MailDeleted(_) => super::SyncStream::GmailHistory,
        fabric::GoogleEvent::CalendarEventCreated(_)
        | fabric::GoogleEvent::CalendarEventUpdated(_)
        | fabric::GoogleEvent::CalendarEventDeleted(_) => super::SyncStream::Calendar,
        fabric::GoogleEvent::DriveFileCreated(_)
        | fabric::GoogleEvent::DriveFileUpdated(_)
        | fabric::GoogleEvent::DriveFileDeleted(_) => super::SyncStream::DriveChanges,
    }
}
