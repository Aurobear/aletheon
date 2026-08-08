//! Event-sourced SessionAppendStore adapter.
//!
//! All production Session/Item mutations pass through the canonical event
//! spine and deterministic reducers before the compatibility SQLite read model
//! is materialized.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use fabric::{
    AppendOutcome, EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity,
    EventPayload, EventSpine, EventTreeId, EventVisibility, ItemId, ItemPayload, ItemRecord,
    MessageId, NamespaceId, PrincipalId, SchemaId, SessionAppendStore, SessionForkedEvent,
    SessionId, SessionPrincipalBoundEvent, SessionReadStore, SessionRecord, SpineEvent,
    UnsequencedEvent, SESSION_SCHEMA_VERSION,
};
use uuid::Uuid;

use crate::adapters::events::session_projection::SessionProjection;
use crate::adapters::session::projection_store::SessionProjectionStore;
use crate::application::event_projection::EventProjectionSink;

const SESSION_EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x01b2f7f1_0d98_441a_a30e_4f637b27be55);
const FORK_ITEM_NAMESPACE: Uuid = Uuid::from_u128(0x97223947_4cbc_4e93_94fa_a71798f64a30);
const RECONCILIATION_PAGE_SIZE: usize = 256;
const APPEND_SEQUENCE_RETRY_LIMIT: u32 = 32;

static APPEND_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static APPEND_SEQUENCE_RETRIES: AtomicU64 = AtomicU64::new(0);
static APPEND_CONFLICTS: AtomicU64 = AtomicU64::new(0);
static APPEND_IDEMPOTENT_REPLAYS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SessionAppendMetrics {
    pub attempts: u64,
    pub sequence_retries: u64,
    pub conflicts: u64,
    pub idempotent_replays: u64,
}

pub fn session_append_metrics() -> SessionAppendMetrics {
    SessionAppendMetrics {
        attempts: APPEND_ATTEMPTS.load(Ordering::Acquire),
        sequence_retries: APPEND_SEQUENCE_RETRIES.load(Ordering::Acquire),
        conflicts: APPEND_CONFLICTS.load(Ordering::Acquire),
        idempotent_replays: APPEND_IDEMPOTENT_REPLAYS.load(Ordering::Acquire),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionEventReconcileReport {
    pub scanned: u64,
    pub materialized: u64,
}

/// Replay the committed event-spine prefix into deterministic projections and
/// the compatibility Session read model. Every operation is idempotent, so a
/// restart can safely replay events committed before any prior crash point.
pub async fn reconcile_committed_session_events(
    event_spine: &dyn EventSpine,
    event_projections: &dyn EventProjectionSink,
    read_model: &dyn SessionProjectionStore,
) -> Result<SessionEventReconcileReport> {
    let through_row_id = event_spine.committed_watermark()?;
    let mut after_row_id = 0;
    let mut report = SessionEventReconcileReport::default();

    loop {
        let page = event_spine.read_committed_page(
            after_row_id,
            through_row_id,
            RECONCILIATION_PAGE_SIZE,
        )?;
        if page.is_empty() {
            break;
        }
        for (row_id, event) in page {
            let session_event = is_session_materialization_event(&event);
            apply_event_projections(event_projections, &event, session_event)?;
            report.scanned += 1;
            if session_event {
                SessionProjection::materialize(read_model, &event).await?;
                report.materialized += 1;
            }
            after_row_id = row_id;
        }
    }

    Ok(report)
}

pub struct EventSourcedSessionStore {
    read_model: Arc<dyn SessionProjectionStore>,
    event_spine: Arc<dyn EventSpine>,
    event_projections: Arc<dyn EventProjectionSink>,
    /// Short-lived registry of per-session writer locks. The registry mutex is
    /// never held across I/O; different sessions therefore do not queue behind
    /// one daemon-wide writer guard.
    writer_locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl EventSourcedSessionStore {
    pub fn new(
        read_model: Arc<dyn SessionProjectionStore>,
        event_spine: Arc<dyn EventSpine>,
        event_projections: Arc<dyn EventProjectionSink>,
    ) -> Self {
        Self {
            read_model,
            event_spine,
            event_projections,
            writer_locks: Mutex::new(HashMap::new()),
        }
    }

    fn writer_lock(&self, session_id: &SessionId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .writer_locks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&session_id.0).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(session_id.0.clone(), Arc::downgrade(&lock));
        lock
    }

    async fn lock_sessions(
        &self,
        sessions: impl IntoIterator<Item = SessionId>,
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut sessions = sessions.into_iter().collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.0.cmp(&right.0));
        sessions.dedup();
        let locks = sessions
            .iter()
            .map(|session| self.writer_lock(session))
            .collect::<Vec<_>>();
        let mut guards = Vec::with_capacity(locks.len());
        for lock in locks {
            guards.push(lock.lock_owned().await);
        }
        guards
    }

    async fn append_and_materialize(&self, input: UnsequencedEvent) -> Result<SpineEvent> {
        let event = self.event_spine.append(input)?;
        apply_event_projections(self.event_projections.as_ref(), &event, true)?;
        SessionProjection::materialize(self.read_model.as_ref(), &event).await?;
        Ok(event)
    }

    fn event(
        schema: &'static str,
        session_id: &SessionId,
        stable_key: &str,
        visibility: EventVisibility,
        payload: serde_json::Value,
    ) -> UnsequencedEvent {
        let event_id = EventId(Uuid::new_v5(
            &SESSION_EVENT_NAMESPACE,
            stable_key.as_bytes(),
        ));
        let mut envelope = EnvelopeV2::new(
            SchemaId(schema.into()),
            EnvelopeV2Target("session-command".into()),
            EnvelopeV2Target(format!("session:{}", session_id.0)),
            EnvelopeV2Delivery::Direct,
            NamespaceId(format!("session:{}", session_id.0)),
            payload.clone(),
        );
        envelope.id = MessageId(event_id.0);
        UnsequencedEvent {
            tree_id: EventTreeId::for_root_session(&session_id.0),
            event_id,
            parent: None,
            identity: EventIdentity {
                root_session_id: session_id.0.clone(),
                session_id: session_id.0.clone(),
                agent_id: None,
            },
            envelope,
            visibility,
            payload: EventPayload::Inline { value: payload },
        }
    }

    fn item_visibility(payload: &ItemPayload) -> EventVisibility {
        match payload {
            ItemPayload::UserMessage { .. }
            | ItemPayload::AssistantMessage { .. }
            | ItemPayload::ToolCall { .. }
            | ItemPayload::ToolResult { .. } => EventVisibility::ModelVisible,
            ItemPayload::ContextProjection { .. }
            | ItemPayload::SystemNotice { .. }
            | ItemPayload::CapabilityReceipt { .. }
            | ItemPayload::RobotEpisodeReceipt { .. }
            | ItemPayload::EvaluationReceiptRef { .. }
            | ItemPayload::ModelContextProjection { .. }
            | ItemPayload::ContextBudgetProjection { .. }
            | ItemPayload::ContextCompactionProjection { .. }
            | ItemPayload::InferenceReceipt { .. }
            | ItemPayload::TaskProjection { .. }
            | ItemPayload::TurnRecovery { .. }
            | ItemPayload::TurnSettlement { .. } => EventVisibility::Control,
        }
    }
}

#[async_trait]
impl SessionReadStore for EventSourcedSessionStore {
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>> {
        self.read_model.load_session(session).await
    }

    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>> {
        self.read_model.load_items(session, after).await
    }

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRecord>> {
        self.read_model.list_sessions(limit).await
    }

    async fn list_session_ids(&self) -> Result<Vec<SessionId>> {
        self.read_model.list_session_ids().await
    }

    async fn principal_for(&self, session: &SessionId) -> Result<Option<PrincipalId>> {
        self.read_model.principal_for(session).await
    }
}

fn apply_event_projections(
    event_projections: &dyn EventProjectionSink,
    event: &SpineEvent,
    public_failure_is_fatal: bool,
) -> Result<()> {
    let report = event_projections.project(event);
    for lag in report.lags.iter().filter(|lag| lag.pending_events > 0) {
        tracing::warn!(
            projection = %lag.projection,
            input_sequence = lag.input_sequence,
            through_sequence = lag.through_sequence,
            pending_events = lag.pending_events,
            "event projection is behind its input watermark"
        );
    }
    for poison in &report.poisons {
        tracing::warn!(
            projection = %poison.projection,
            event_id = %poison.event_id,
            sequence = poison.sequence,
            error = %poison.error,
            "event projection poison recorded"
        );
    }
    let mut public_failure = None;
    for failure in report.failures {
        if failure.projection == "public-session" && public_failure_is_fatal {
            public_failure = Some(failure.error);
        } else {
            tracing::warn!(
                projection = %failure.projection,
                error = %failure.error,
                "event projection failed; unrelated reducers continued"
            );
        }
    }
    if let Some(error) = public_failure {
        bail!("public Session projection failed: {error}");
    }
    Ok(())
}

fn is_session_materialization_event(event: &SpineEvent) -> bool {
    event.envelope.source.0 == "session-command"
        && matches!(
            event.schema.0.as_str(),
            SchemaId::EVENT_SESSION_CREATED_V1
                | SchemaId::EVENT_SESSION_FORKED_V1
                | SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1
                | SchemaId::TURN_EVENT_V1
        )
}

#[async_trait]
impl SessionAppendStore for EventSourcedSessionStore {
    async fn create(&self, session: SessionRecord) -> Result<()> {
        if session.schema_version != SESSION_SCHEMA_VERSION {
            bail!(
                "unsupported session schema version {}",
                session.schema_version
            );
        }
        let _guards = self.lock_sessions([session.id.clone()]).await;
        let payload = serde_json::to_value(&session)?;
        self.append_and_materialize(Self::event(
            SchemaId::EVENT_SESSION_CREATED_V1,
            &session.id,
            &format!("session-created:{}", session.id.0),
            EventVisibility::Control,
            payload,
        ))
        .await?;
        Ok(())
    }

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome> {
        if item.schema_version != SESSION_SCHEMA_VERSION
            || &item.session_id != session
            || item.sequence != expected_sequence
        {
            bail!("Session item does not match append contract");
        }
        if matches!(
            &item.payload,
            ItemPayload::TaskProjection { fact }
                if fact.schema_version != fabric::TASK_PROJECTION_FACT_SCHEMA_VERSION
        ) {
            bail!("unsupported Task projection fact schema version");
        }
        let session_lock = self.writer_lock(session);
        for attempt in 0..=APPEND_SEQUENCE_RETRY_LIMIT {
            APPEND_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            let guard = session_lock.lock().await;
            // Head-based admission: distinguish an idempotent replay from a
            // sequence conflict using O(1) lookups instead of scanning the full
            // session history (S1-AUDIT-001/002).
            let replay = match self.read_model.item_by_id(session, &item.id).await? {
                Some(existing) => {
                    if existing != item {
                        APPEND_CONFLICTS.fetch_add(1, Ordering::Relaxed);
                        bail!("item id retry conflicts with persisted content");
                    }
                    true
                }
                None => false,
            };
            if !replay {
                let next = self.read_model.next_sequence(session).await?.unwrap_or(1);
                if next < expected_sequence && attempt < APPEND_SEQUENCE_RETRY_LIMIT {
                    APPEND_SEQUENCE_RETRIES.fetch_add(1, Ordering::Relaxed);
                    drop(guard);
                    let delay_ms = 1u64 << attempt.min(4);
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                if next != expected_sequence {
                    APPEND_CONFLICTS.fetch_add(1, Ordering::Relaxed);
                    bail!("sequence conflict: expected {expected_sequence}, current {next}");
                }
            }
            let visibility = Self::item_visibility(&item.payload);
            let payload = serde_json::to_value(&item)?;
            // Idempotent spine append + materialization: a replay re-uses the same
            // deterministic event id (spine returns the existing event) and the
            // read model's `AlreadyPresent`, so no duplicate is committed.
            self.append_and_materialize(Self::event(
                SchemaId::TURN_EVENT_V1,
                session,
                &format!("session-item:{}", item.id.0),
                visibility,
                payload,
            ))
            .await?;
            if replay {
                APPEND_IDEMPOTENT_REPLAYS.fetch_add(1, Ordering::Relaxed);
                return Ok(AppendOutcome::AlreadyPresent);
            }
            return Ok(AppendOutcome::Appended);
        }
        unreachable!("bounded append retry loop always returns or errors")
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()> {
        let _guards = self.lock_sessions([parent.clone(), child.id.clone()]).await;
        let parent_link = child
            .parent
            .as_ref()
            .context("fork child missing parent metadata")?;
        if &parent_link.session_id != parent || parent_link.through_sequence != through_sequence {
            bail!("fork metadata does not match request");
        }
        if self.read_model.load_session(parent).await?.is_none() {
            bail!("parent Session does not exist");
        }
        let parent_items = self.read_model.load_items(parent, None).await?;
        if through_sequence > 0
            && parent_items
                .last()
                .is_none_or(|item| through_sequence > item.sequence)
        {
            bail!("parent sequence {through_sequence} does not exist");
        }
        let inherited_items = parent_items
            .into_iter()
            .filter(|item| item.sequence <= through_sequence)
            .map(|mut item| {
                item.id = ItemId(Uuid::new_v5(
                    &FORK_ITEM_NAMESPACE,
                    format!("{}:{}", child.id.0, item.id.0).as_bytes(),
                ));
                item.session_id = child.id.clone();
                item
            })
            .collect();
        let fork = SessionForkedEvent {
            parent_session_id: parent.clone(),
            through_sequence,
            child: child.clone(),
            inherited_items,
        };
        let payload = serde_json::to_value(&fork)?;
        self.append_and_materialize(Self::event(
            SchemaId::EVENT_SESSION_FORKED_V1,
            &child.id,
            &format!("session-forked:{}", child.id.0),
            EventVisibility::Control,
            payload,
        ))
        .await?;
        Ok(())
    }

    async fn bind_principal(&self, session: &SessionId, principal: &PrincipalId) -> Result<()> {
        let _guards = self.lock_sessions([session.clone()]).await;
        if self.read_model.load_session(session).await?.is_none() {
            bail!("Session does not exist");
        }
        if let Some(owner) = self.read_model.principal_for(session).await? {
            if owner != *principal {
                bail!("session is already owned by another principal");
            }
        }
        let binding = SessionPrincipalBoundEvent {
            session_id: session.clone(),
            principal: principal.clone(),
        };
        self.append_and_materialize(Self::event(
            SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1,
            session,
            &format!("session-principal:{}:{}", session.0, principal.0),
            EventVisibility::Control,
            serde_json::to_value(binding)?,
        ))
        .await?;
        Ok(())
    }
}
