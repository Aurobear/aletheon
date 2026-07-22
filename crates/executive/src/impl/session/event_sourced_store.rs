//! Event-sourced SessionAppendStore adapter.
//!
//! All production Session/Item mutations pass through the canonical event
//! spine and deterministic reducers before the compatibility SQLite read model
//! is materialized.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use fabric::{
    AppendOutcome, EnvelopeV2, EnvelopeV2Delivery, EnvelopeV2Target, EventId, EventIdentity,
    EventPayload, EventSpine, EventTreeId, EventVisibility, ItemId, ItemPayload, ItemRecord,
    MessageId, NamespaceId, SchemaId, SessionAppendStore, SessionForkedEvent, SessionId,
    SessionRecord, SpineEvent, UnsequencedEvent, SESSION_SCHEMA_VERSION,
};
use uuid::Uuid;

use crate::application::event_projection::EventProjectionSink;
use crate::r#impl::events::session_projection::SessionProjection;
use crate::r#impl::events::SqliteEventSpine;

const SESSION_EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x01b2f7f1_0d98_441a_a30e_4f637b27be55);
const FORK_ITEM_NAMESPACE: Uuid = Uuid::from_u128(0x97223947_4cbc_4e93_94fa_a71798f64a30);
const RECONCILIATION_PAGE_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionEventReconcileReport {
    pub scanned: u64,
    pub materialized: u64,
}

/// Replay the committed event-spine prefix into deterministic projections and
/// the compatibility Session read model. Every operation is idempotent, so a
/// restart can safely replay events committed before any prior crash point.
pub async fn reconcile_committed_session_events(
    event_spine: &SqliteEventSpine,
    event_projections: &dyn EventProjectionSink,
    read_model: &dyn SessionAppendStore,
) -> Result<SessionEventReconcileReport> {
    let through_row_id = event_spine.committed_row_watermark()?;
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
    read_model: Arc<dyn SessionAppendStore>,
    event_spine: Arc<dyn EventSpine>,
    event_projections: Arc<dyn EventProjectionSink>,
    writer: tokio::sync::Mutex<()>,
}

impl EventSourcedSessionStore {
    pub fn new(
        read_model: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
        event_projections: Arc<dyn EventProjectionSink>,
    ) -> Self {
        Self {
            read_model,
            event_spine,
            event_projections,
            writer: tokio::sync::Mutex::new(()),
        }
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
            ItemPayload::ContextProjection { .. } | ItemPayload::SystemNotice { .. } => {
                EventVisibility::Control
            }
        }
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
        let _guard = self.writer.lock().await;
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
        let _guard = self.writer.lock().await;
        let items = self.read_model.load_items(session, None).await?;
        if let Some(existing) = items.iter().find(|existing| existing.id == item.id) {
            if existing != &item {
                bail!("item id retry conflicts with persisted content");
            }
        } else {
            let next = items.last().map_or(1, |current| current.sequence + 1);
            if next != expected_sequence {
                bail!("sequence conflict: expected {expected_sequence}, current {next}");
            }
        }
        let already_present = items.iter().any(|existing| existing.id == item.id);
        let visibility = Self::item_visibility(&item.payload);
        let payload = serde_json::to_value(&item)?;
        self.append_and_materialize(Self::event(
            SchemaId::TURN_EVENT_V1,
            session,
            &format!("session-item:{}", item.id.0),
            visibility,
            payload,
        ))
        .await?;
        Ok(if already_present {
            AppendOutcome::AlreadyPresent
        } else {
            AppendOutcome::Appended
        })
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()> {
        let _guard = self.writer.lock().await;
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

    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>> {
        self.read_model.load_session(session).await
    }

    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>> {
        self.read_model.load_items(session, after).await
    }
}
