//! Resume, fork, interrupt, and replay over canonical session history.

use std::{collections::HashSet, sync::Arc};

use anyhow::{bail, Result};
use fabric::{
    AppendOutcome, ContentBlock, ItemId, ItemPayload, ItemRecord, Message, Role,
    SessionAppendStore, SessionFork, SessionId, SessionRecord, SessionStatus, TurnId,
    SESSION_SCHEMA_VERSION,
};
use tokio::sync::Mutex;

use crate::r#impl::session::canonical_store::project_messages;

use super::turn_coordinator::{ActiveTurn, ActiveTurnKey};

pub struct ResumeResult {
    pub session: SessionRecord,
    pub next_sequence: u64,
    pub messages: Vec<fabric::Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Interrupted,
    AlreadyTerminal,
}

pub struct SessionService {
    store: Arc<dyn SessionAppendStore>,
    active: Arc<Mutex<std::collections::HashMap<ActiveTurnKey, ActiveTurn>>>,
    interrupted: Mutex<HashSet<String>>,
}

impl SessionService {
    pub fn new(
        store: Arc<dyn SessionAppendStore>,
        active: Arc<Mutex<std::collections::HashMap<ActiveTurnKey, ActiveTurn>>>,
    ) -> Self {
        Self {
            store,
            active,
            interrupted: Mutex::new(HashSet::new()),
        }
    }

    pub async fn resume(&self, session_id: &SessionId) -> Result<ResumeResult> {
        self.try_resume(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session not found"))
    }

    pub async fn try_resume(&self, session_id: &SessionId) -> Result<Option<ResumeResult>> {
        let Some(session) = self.store.load_session(session_id).await? else {
            return Ok(None);
        };
        let items = self.store.load_items(session_id, None).await?;
        let next_sequence = items.last().map_or(1, |item| item.sequence + 1);
        Ok(Some(ResumeResult {
            session,
            next_sequence,
            messages: project_messages(&items)?,
        }))
    }

    pub async fn items(&self, session_id: &SessionId) -> Result<Vec<ItemRecord>> {
        if self.store.load_session(session_id).await?.is_none() {
            bail!("session not found");
        }
        self.store.load_items(session_id, None).await
    }

    /// Build the transport-neutral snapshot used by the versioned daemon
    /// protocol. The cursor names the last durable item, so reconnect can
    /// resume strictly after it without relying on process-local stream state.
    pub async fn protocol_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<fabric::protocol::client::UiSnapshot> {
        let items = self.items(session_id).await?;
        let cursor = items
            .last()
            .map(item_cursor)
            .unwrap_or_else(fabric::protocol::client::EventCursor::origin);
        Ok(fabric::protocol::client::UiSnapshot {
            session_id: session_id.clone(),
            cursor,
            provider: None,
            model: None,
            items,
            approvals: Vec::new(),
            agents: Vec::new(),
        })
    }

    /// Replay durable item terminals strictly after an authenticated cursor.
    /// A non-origin cursor must name the item at its sequence; this prevents a
    /// stale or forged `(sequence,event_id)` pair from skipping history.
    pub async fn protocol_events_after(
        &self,
        session_id: &SessionId,
        after: &fabric::protocol::client::EventCursor,
    ) -> Result<Vec<fabric::protocol::client::ClientEvent>> {
        if after.sequence == 0 {
            if after.event_id.is_some() {
                bail!("origin cursor cannot carry an event_id");
            }
        } else {
            let anchor = self
                .store
                .load_items(session_id, Some(after.sequence.saturating_sub(1)))
                .await?
                .into_iter()
                .next()
                .filter(|item| item.sequence == after.sequence)
                .ok_or_else(|| anyhow::anyhow!("cursor sequence is not present"))?;
            let anchor_event_id = anchor.id.0.to_string();
            if after.event_id.as_deref() != Some(anchor_event_id.as_str()) {
                bail!("cursor event_id does not match durable item");
            }
        }

        let items = self
            .store
            .load_items(session_id, Some(after.sequence))
            .await?;
        Ok(items.into_iter().map(item_terminal_event).collect())
    }

    /// Persist lifecycle-provided workspace context into canonical history
    /// before it is used for model projection. The Fabric validator is the
    /// single authority for effect bounds and phase legality.
    pub async fn persist_context_fragments(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        phase: fabric::types::lifecycle::LifecyclePhase,
        fragments: Vec<(String, String)>,
    ) -> Result<usize> {
        if fragments.is_empty() {
            return Ok(0);
        }
        let effects = fragments
            .into_iter()
            .map(|(source, content)| {
                fabric::types::lifecycle::LifecycleEffect::AddContextFragment { source, content }
            })
            .collect::<Vec<_>>();
        let items = self.items(session_id).await?;
        let mut sequence = items.last().map_or(1, |item| item.sequence + 1);
        super::context_fragment::inject_context_fragments(
            self.store.as_ref(),
            session_id,
            turn_id,
            &mut sequence,
            phase,
            &effects,
        )
        .await
    }

    /// Ensure a legacy session has a canonical Session/Turn/Item projection.
    ///
    /// Import is intentionally append-only: an existing canonical history is
    /// never rewritten from the compatibility journal.
    pub async fn ensure_legacy_projection(
        &self,
        session_id: &SessionId,
        messages: &[Message],
        created_at_ms: u64,
    ) -> Result<()> {
        if self.store.load_session(session_id).await?.is_none() {
            self.store
                .create(SessionRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: session_id.clone(),
                    parent: None,
                    created_at_ms,
                    status: SessionStatus::Active,
                })
                .await?;
        }
        if !self.store.load_items(session_id, None).await?.is_empty() {
            return Ok(());
        }

        let mut sequence = 1;
        for message in messages {
            let turn_id = TurnId::new();
            for payload in legacy_message_payloads(message) {
                let item = ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id,
                    sequence,
                    created_at_ms,
                    payload,
                };
                match self.store.append(session_id, sequence, item).await? {
                    AppendOutcome::Appended | AppendOutcome::AlreadyPresent => sequence += 1,
                }
            }
        }
        Ok(())
    }

    pub async fn fork(&self, parent: &SessionId, through_sequence: u64) -> Result<SessionRecord> {
        let child = SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: SessionId(uuid::Uuid::new_v4().to_string()),
            parent: Some(SessionFork {
                session_id: parent.clone(),
                through_sequence,
            }),
            created_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
            status: SessionStatus::Active,
        };
        self.store
            .fork(parent, through_sequence, child.clone())
            .await?;
        Ok(child)
    }

    pub async fn replay(
        &self,
        session_id: &SessionId,
        after: Option<u64>,
    ) -> Result<Vec<fabric::Message>> {
        if self.store.load_session(session_id).await?.is_none() {
            bail!("session not found");
        }
        project_messages(&self.store.load_items(session_id, after).await?)
    }

    pub async fn interrupt(&self, session_id: &SessionId) -> Result<InterruptOutcome> {
        let mut interrupted = self.interrupted.lock().await;
        if interrupted.contains(&session_id.0) {
            return Ok(InterruptOutcome::AlreadyTerminal);
        }
        // Legacy session RPCs do not yet carry a principal. This compatibility
        // lookup is removed when those RPCs move to PrincipalContext in M3.
        let active = self
            .active
            .lock()
            .await
            .iter()
            .find(|(key, _)| key.thread_id.0 == session_id.0)
            .map(|(_, active)| active.clone());
        let Some(active) = active else {
            return Ok(InterruptOutcome::AlreadyTerminal);
        };
        active.cancel.cancel();
        interrupted.insert(session_id.0.clone());
        Ok(InterruptOutcome::Interrupted)
    }
}

fn item_cursor(item: &ItemRecord) -> fabric::protocol::client::EventCursor {
    fabric::protocol::client::EventCursor {
        sequence: item.sequence,
        event_id: Some(item.id.0.to_string()),
    }
}

fn item_terminal_event(item: ItemRecord) -> fabric::protocol::client::ClientEvent {
    let (phase, error) = match &item.payload {
        ItemPayload::ToolResult {
            content,
            is_error: true,
            ..
        } => (
            fabric::protocol::client::ItemPhase::Failed,
            Some(content.clone()),
        ),
        _ => (fabric::protocol::client::ItemPhase::Completed, None),
    };
    fabric::protocol::client::ClientEvent::Item(fabric::protocol::client::ItemEvent {
        cursor: item_cursor(&item),
        item_id: item.id.0.to_string(),
        phase,
        delta: None,
        item: Some(item),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_context_fragment_is_bounded_and_durable() {
        let store: Arc<dyn SessionAppendStore> = Arc::new(
            crate::r#impl::session::canonical_store::CanonicalSessionStore::open(":memory:")
                .unwrap(),
        );
        let session_id = SessionId("context-session".into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 1,
                status: SessionStatus::Active,
            })
            .await
            .unwrap();
        let service = SessionService::new(store, Arc::new(Mutex::new(Default::default())));
        let persisted = service
            .persist_context_fragments(
                &session_id,
                TurnId::new(),
                fabric::types::lifecycle::LifecyclePhase::BeforeTurnInput,
                vec![("workspace".into(), "branch=feature".into())],
            )
            .await
            .unwrap();
        assert_eq!(persisted, 1);
        let items = service.items(&session_id).await.unwrap();
        assert!(matches!(
            &items[0].payload,
            ItemPayload::SystemNotice { content }
                if content.contains("source=workspace") && content.contains("branch=feature")
        ));
    }
}

fn legacy_message_payloads(message: &Message) -> Vec<ItemPayload> {
    let mut payloads = Vec::new();
    for block in &message.content {
        let payload = match block {
            ContentBlock::Text { text } => match message.role {
                Role::User => ItemPayload::UserMessage {
                    content: text.clone(),
                },
                Role::Assistant => ItemPayload::AssistantMessage {
                    content: text.clone(),
                },
                Role::System => ItemPayload::SystemNotice {
                    content: text.clone(),
                },
            },
            ContentBlock::ToolUse { id, name, input } => ItemPayload::ToolCall {
                call_id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            },
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => ItemPayload::ToolResult {
                call_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
                permit_id: None,
                audit_id: None,
            },
            ContentBlock::System { text, .. } => ItemPayload::SystemNotice {
                content: text.clone(),
            },
            ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => continue,
        };
        payloads.push(payload);
    }
    payloads
}
