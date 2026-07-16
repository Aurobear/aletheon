use std::collections::BTreeMap;

use fabric::{
    EventPayload, EventVisibility, ItemRecord, SessionAppendStore, SessionForkedEvent, SessionId,
    SessionRecord, SpineEvent,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::service::event_projection::{EventProjection, ProjectionDescriptor, ProjectionError};

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
        if event.visibility == EventVisibility::Sensitive {
            return Ok(());
        }
        match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_SESSION_CREATED_V1 => {
                let session: SessionRecord = decode_inline_anyhow(event)?;
                store.create(session).await
            }
            fabric::SchemaId::EVENT_SESSION_FORKED_V1 => {
                let fork: SessionForkedEvent = decode_inline_anyhow(event)?;
                store.create(fork.child.clone()).await?;
                for item in fork.inherited_items {
                    let sequence = item.sequence;
                    let session_id = item.session_id.clone();
                    store.append(&session_id, sequence, item).await?;
                }
                Ok(())
            }
            fabric::SchemaId::TURN_EVENT_V1 => {
                let item: ItemRecord = decode_inline_anyhow(event)?;
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
        let record: SessionRecord = decode_inline(event)?;
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
        let fork: SessionForkedEvent = decode_inline(event)?;
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
        let item: ItemRecord = decode_inline(event)?;
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
        if event.visibility == EventVisibility::Sensitive {
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
