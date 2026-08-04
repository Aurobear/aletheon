use std::collections::BTreeMap;

use fabric::{
    EventPayload, EventVisibility, ItemRecord, SessionAppendStore, SessionForkedEvent, SessionId,
    SessionRecord, SpineEvent, SESSION_SCHEMA_VERSION,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::application::event_projection::{
    EventProjection, ProjectionDescriptor, ProjectionError,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionState {
    pub sessions: BTreeMap<String, PublicSessionView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<SessionRecord>,
    pub turns: BTreeMap<String, Vec<u64>>,
    pub items: Vec<ItemRecord>,
}

pub struct SessionProjection;

impl SessionProjection {
    /// Materialize one already-persisted spine event into the compatibility
    /// SessionAppendStore read model. Production handlers never pass an
    /// independently assembled Session/Item value to that store.
    pub async fn materialize(
        store: &dyn SessionAppendStore,
        event: &SpineEvent,
    ) -> anyhow::Result<()> {
        if event.visibility == EventVisibility::Sensitive
            || is_legacy_evaluation_projection_event(event)
        {
            return Ok(());
        }
        match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_SESSION_CREATED_V1 => {
                let session = current_session(decode_inline_anyhow(event)?)?;
                materialize_session_creation(store, session).await
            }
            fabric::SchemaId::EVENT_SESSION_FORKED_V1 => {
                let fork = current_fork(decode_inline_anyhow(event)?)?;
                materialize_session_creation(store, fork.child.clone()).await?;
                for item in fork.inherited_items {
                    let sequence = item.sequence;
                    let session_id = item.session_id.clone();
                    store.append(&session_id, sequence, item).await?;
                }
                Ok(())
            }
            fabric::SchemaId::TURN_EVENT_V1 => {
                // A few legacy events were written under the turn schema with a
                // non-item payload (e.g. admin profile switches). Tolerate them
                // instead of poisoning the whole public-session projection: skip
                // the event with a warning, keep processing the rest.
                let item = match decode_inline_anyhow::<ItemRecord>(event)
                    .and_then(current_item)
                {
                    Ok(item) => item,
                    Err(error) => {
                        tracing::warn!(
                            event_id = %event.position.event_id.0,
                            %error,
                            "skipping turn-schema event with non-item payload"
                        );
                        return Ok(());
                    }
                };
                let sequence = item.sequence;
                let session_id = item.session_id.clone();
                store.append(&session_id, sequence, item).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn apply_session_created(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        let record = current_session(decode_inline(event)?).map_err(ProjectionError::Storage)?;
        if record.id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Session identity differs from spine"));
        }
        let view = state.sessions.entry(record.id.0.clone()).or_default();
        if view
            .record
            .as_ref()
            .is_some_and(|current| current != &record)
        {
            return Err(invalid("Session creation conflicts with prior event"));
        }
        view.record = Some(record);
        Ok(())
    }

    fn apply_session_forked(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        let fork = current_fork(decode_inline(event)?).map_err(ProjectionError::Storage)?;
        if fork.child.id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Fork child identity differs from spine"));
        }
        let parent = fork
            .child
            .parent
            .as_ref()
            .ok_or_else(|| invalid("Fork child is missing parent metadata"))?;
        if parent.session_id != fork.parent_session_id
            || parent.through_sequence != fork.through_sequence
        {
            return Err(invalid("Fork payload and child metadata disagree"));
        }
        validate_items(&fork.child.id, &fork.inherited_items)?;
        if fork
            .inherited_items
            .last()
            .is_some_and(|item| item.sequence > fork.through_sequence)
        {
            return Err(invalid("Fork inherited items exceed boundary"));
        }
        let mut view = PublicSessionView {
            record: Some(fork.child.clone()),
            ..Default::default()
        };
        for item in fork.inherited_items {
            view.turns
                .entry(item.turn_id.0.to_string())
                .or_default()
                .push(item.sequence);
            view.items.push(item);
        }
        match state.sessions.get(&fork.child.id.0) {
            Some(current) if current != &view => {
                Err(invalid("Fork event conflicts with prior child projection"))
            }
            _ => {
                state.sessions.insert(fork.child.id.0, view);
                Ok(())
            }
        }
    }

    fn apply_item(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        let item = current_item(decode_inline(event)?).map_err(ProjectionError::Storage)?;
        if item.session_id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Session item identity differs from spine"));
        }
        let session = state.sessions.entry(item.session_id.0.clone()).or_default();
        if session
            .items
            .last()
            .is_some_and(|prior| prior.sequence >= item.sequence)
        {
            return Err(ProjectionError::NonMonotonic {
                previous: session.items.last().unwrap().sequence,
                current: item.sequence,
            });
        }
        session
            .turns
            .entry(item.turn_id.0.to_string())
            .or_default()
            .push(item.sequence);
        session.items.push(item);
        Ok(())
    }
}

async fn materialize_session_creation(
    store: &dyn SessionAppendStore,
    created: SessionRecord,
) -> anyhow::Result<()> {
    let Some(current) = store.load_session(&created.id).await? else {
        return store.create(created).await;
    };
    anyhow::ensure!(
        current.schema_version == created.schema_version
            && current.id == created.id
            && current.parent == created.parent
            && current.created_at_ms == created.created_at_ms,
        "session creation conflicts with persisted immutable content"
    );
    // Session status is a mutable read-model projection (for example startup
    // recovery can mark an interrupted Turn). Replaying the original creation
    // event must preserve that later status rather than treating it as a
    // conflicting create retry.
    Ok(())
}

impl EventProjection for SessionProjection {
    type State = PublicSessionState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "public-session",
            version: 1,
            accepted_schemas: &[
                fabric::SchemaId::EVENT_SESSION_CREATED_V1,
                fabric::SchemaId::EVENT_SESSION_FORKED_V1,
                fabric::SchemaId::TURN_EVENT_V1,
            ],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        if event.visibility == EventVisibility::Sensitive
            || is_legacy_evaluation_projection_event(event)
        {
            return Ok(());
        }
        match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_SESSION_CREATED_V1 => Self::apply_session_created(state, event),
            fabric::SchemaId::EVENT_SESSION_FORKED_V1 => Self::apply_session_forked(state, event),
            fabric::SchemaId::TURN_EVENT_V1 => Self::apply_item(state, event),
            _ => Ok(()),
        }
    }
}

/// Releases before the dedicated evaluation schema incorrectly published
/// domain evaluation observations as `turn.event/v1`. Keep those immutable
/// historical events replayable without treating their non-ItemRecord payload
/// as public Session data. New observations use `evaluation_observed/v1`.
fn is_legacy_evaluation_projection_event(event: &SpineEvent) -> bool {
    event.schema.0 == fabric::SchemaId::TURN_EVENT_V1
        && event.envelope.source.0 == "evaluation-projection"
        && matches!(
            &event.payload,
            EventPayload::Inline { value }
                if value
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| kind.starts_with("evaluation.") && kind.ends_with(".observed"))
        )
}

fn decode_inline<T: DeserializeOwned>(event: &SpineEvent) -> Result<T, ProjectionError> {
    let EventPayload::Inline { value } = &event.payload else {
        return Err(invalid("Session projection requires an inline payload"));
    };
    serde_json::from_value(value.clone())
        .map_err(anyhow::Error::from)
        .map_err(ProjectionError::Storage)
}

fn decode_inline_anyhow<T: DeserializeOwned>(event: &SpineEvent) -> anyhow::Result<T> {
    let EventPayload::Inline { value } = &event.payload else {
        anyhow::bail!("Session projection requires an inline payload");
    };
    Ok(serde_json::from_value(value.clone())?)
}

fn current_session(mut session: SessionRecord) -> anyhow::Result<SessionRecord> {
    ensure_supported_record_version(session.schema_version, "session")?;
    session.schema_version = SESSION_SCHEMA_VERSION;
    Ok(session)
}

fn current_item(mut item: ItemRecord) -> anyhow::Result<ItemRecord> {
    ensure_supported_record_version(item.schema_version, "item")?;
    item.schema_version = SESSION_SCHEMA_VERSION;
    Ok(item)
}

fn current_fork(mut fork: SessionForkedEvent) -> anyhow::Result<SessionForkedEvent> {
    fork.child = current_session(fork.child)?;
    fork.inherited_items = fork
        .inherited_items
        .into_iter()
        .map(current_item)
        .collect::<anyhow::Result<_>>()?;
    Ok(fork)
}

fn ensure_supported_record_version(version: u16, kind: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=SESSION_SCHEMA_VERSION).contains(&version),
        "unsupported {kind} schema version {version}"
    );
    Ok(())
}

fn validate_items(session: &SessionId, items: &[ItemRecord]) -> Result<(), ProjectionError> {
    let mut prior = 0;
    for item in items {
        if &item.session_id != session || item.sequence != prior + 1 {
            return Err(invalid(
                "Fork inherited items are not a contiguous child view",
            ));
        }
        prior = item.sequence;
    }
    Ok(())
}

fn invalid(message: &str) -> ProjectionError {
    ProjectionError::InvalidDescriptor(message.into())
}
