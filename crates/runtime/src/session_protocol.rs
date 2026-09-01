//! Runtime-owned protocol event persistence port.
//!
//! Runtime defines reconnect, cursor, and dedupe semantics. Concrete storage
//! belongs to an adapter; the in-memory implementation is for process-local
//! composition and unit tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use anyhow::Result;
use contracts::protocol::client::{
    ClientEvent, EventCursor, ItemEvent, ItemPhase, TransientApprovalScopeSubject,
};
use contracts::{ItemRecord, SessionId, TurnId};

#[derive(Clone)]
pub struct ProtocolItemWrite {
    pub session_id: SessionId,
    pub item_id: String,
    pub phase: ItemPhase,
    pub delta: Option<String>,
    pub item: Option<ItemRecord>,
    pub error: Option<String>,
    pub dedupe_key: Option<String>,
    /// Canonical Session item sequence represented by this protocol write.
    /// Stores advance the per-session sync watermark atomically with the event.
    pub canonical_sequence: Option<u64>,
}

#[derive(Clone)]
pub struct ProtocolApprovalWrite {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub approval_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    pub detail: Option<String>,
    pub scope_subject: Option<TransientApprovalScopeSubject>,
}

/// Durable protocol journal consumed by the canonical Session use case.
///
/// Implementations must serialize sequence allocation with insert/update and
/// enforce uniqueness of `(session_id, sequence/event_id/dedupe_key)`.
pub trait SessionProtocolEventStore: Send + Sync {
    fn append_item(&self, write: ProtocolItemWrite) -> Result<ClientEvent>;
    fn append_approval(&self, write: ProtocolApprovalWrite) -> Result<ClientEvent>;
    fn event_id_at(&self, session_id: &SessionId, sequence: u64) -> Result<Option<String>>;
    fn events_after(
        &self,
        session_id: &SessionId,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<ClientEvent>>;
    fn tail_cursor(&self, session_id: &SessionId) -> Result<EventCursor>;
    fn canonical_watermark(&self, session_id: &SessionId) -> Result<u64>;
}

#[derive(Clone)]
struct StoredEvent {
    event_id: String,
    event: ClientEvent,
}

#[derive(Default)]
struct MemoryState {
    events: HashMap<String, BTreeMap<u64, StoredEvent>>,
    dedupe: HashMap<(String, String), u64>,
    canonical_watermarks: HashMap<String, u64>,
}

#[derive(Default)]
pub struct InMemorySessionProtocolEventStore {
    state: Mutex<MemoryState>,
}

impl InMemorySessionProtocolEventStore {
    pub fn new() -> Self {
        Self::default()
    }
}

fn cursor(sequence: u64) -> EventCursor {
    EventCursor {
        sequence,
        event_id: Some(uuid::Uuid::new_v4().to_string()),
    }
}

impl SessionProtocolEventStore for InMemorySessionProtocolEventStore {
    fn append_item(&self, write: ProtocolItemWrite) -> Result<ClientEvent> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(key) = write.dedupe_key.as_ref() {
            let lookup = (write.session_id.0.clone(), key.clone());
            if let Some(sequence) = state.dedupe.get(&lookup).copied() {
                let event = {
                    let stored = state
                        .events
                        .get_mut(&write.session_id.0)
                        .and_then(|events| events.get_mut(&sequence))
                        .ok_or_else(|| anyhow::anyhow!("protocol dedupe index is inconsistent"))?;
                    if write.item.is_some() {
                        if let ClientEvent::Item(existing) = &mut stored.event {
                            existing.item = write.item;
                            existing.error = write.error;
                        }
                    }
                    stored.event.clone()
                };
                if let Some(canonical_sequence) = write.canonical_sequence {
                    state
                        .canonical_watermarks
                        .entry(write.session_id.0)
                        .and_modify(|watermark| *watermark = (*watermark).max(canonical_sequence))
                        .or_insert(canonical_sequence);
                }
                return Ok(event);
            }
        }
        let events = state.events.entry(write.session_id.0.clone()).or_default();
        let sequence = events
            .last_key_value()
            .map_or(1, |(sequence, _)| sequence + 1);
        let cursor = cursor(sequence);
        let event = ClientEvent::Item(ItemEvent {
            cursor: cursor.clone(),
            item_id: write.item_id,
            phase: write.phase,
            delta: write.delta,
            item: write.item,
            error: write.error,
        });
        events.insert(
            sequence,
            StoredEvent {
                event_id: cursor.event_id.clone().unwrap_or_default(),
                event: event.clone(),
            },
        );
        if let Some(key) = write.dedupe_key {
            state
                .dedupe
                .insert((write.session_id.0.clone(), key), sequence);
        }
        if let Some(canonical_sequence) = write.canonical_sequence {
            state
                .canonical_watermarks
                .entry(write.session_id.0)
                .and_modify(|watermark| *watermark = (*watermark).max(canonical_sequence))
                .or_insert(canonical_sequence);
        }
        Ok(event)
    }

    fn append_approval(&self, write: ProtocolApprovalWrite) -> Result<ClientEvent> {
        let dedupe_key = format!("approval-request:{}", write.approval_id);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let lookup = (write.session_id.0.clone(), dedupe_key.clone());
        if let Some(sequence) = state.dedupe.get(&lookup).copied() {
            return state
                .events
                .get(&write.session_id.0)
                .and_then(|events| events.get(&sequence))
                .map(|stored| stored.event.clone())
                .ok_or_else(|| anyhow::anyhow!("protocol dedupe index is inconsistent"));
        }
        let events = state.events.entry(write.session_id.0.clone()).or_default();
        let sequence = events
            .last_key_value()
            .map_or(1, |(sequence, _)| sequence + 1);
        let cursor = cursor(sequence);
        let event = ClientEvent::ApprovalRequested {
            cursor: cursor.clone(),
            session_id: write.session_id.clone(),
            turn_id: write.turn_id,
            approval_id: write.approval_id,
            tool: write.tool,
            action_summary: write.action_summary,
            risk_level: write.risk_level,
            detail: write.detail,
            scope_subject: write.scope_subject,
        };
        events.insert(
            sequence,
            StoredEvent {
                event_id: cursor.event_id.clone().unwrap_or_default(),
                event: event.clone(),
            },
        );
        state.dedupe.insert(lookup, sequence);
        Ok(event)
    }

    fn event_id_at(&self, session_id: &SessionId, sequence: u64) -> Result<Option<String>> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state
            .events
            .get(&session_id.0)
            .and_then(|events| events.get(&sequence))
            .map(|stored| stored.event_id.clone()))
    }

    fn events_after(
        &self,
        session_id: &SessionId,
        sequence: u64,
        limit: usize,
    ) -> Result<Vec<ClientEvent>> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state
            .events
            .get(&session_id.0)
            .map_or_else(Vec::new, |events| {
                events
                    .range((sequence + 1)..)
                    .take(limit)
                    .map(|(_, stored)| stored.event.clone())
                    .collect()
            }))
    }

    fn tail_cursor(&self, session_id: &SessionId) -> Result<EventCursor> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state
            .events
            .get(&session_id.0)
            .and_then(|events| events.last_key_value())
            .map_or_else(EventCursor::origin, |(sequence, stored)| EventCursor {
                sequence: *sequence,
                event_id: Some(stored.event_id.clone()),
            }))
    }

    fn canonical_watermark(&self, session_id: &SessionId) -> Result<u64> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state
            .canonical_watermarks
            .get(&session_id.0)
            .copied()
            .unwrap_or(0))
    }
}
