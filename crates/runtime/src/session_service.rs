//! Resume, fork, interrupt, and replay over canonical session history.

use ::contracts::LOCAL_OWNER_PRINCIPAL;
use std::{collections::HashSet, path::Path, sync::Arc};

use ::contracts::{
    AppendOutcome, ContentBlock, ItemId, ItemPayload, ItemRecord, Message, PrincipalId, Role,
    SessionAppendStore, SessionFork, SessionId, SessionRecord, SessionStatus, TaskProjectionFact,
    TurnId, SESSION_SCHEMA_VERSION,
};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use crate::public_session_projection::SessionProjection;
use crate::session_projection::project_messages;

use crate::turn_registry::ActiveTurnRegistry;

const SESSION_EVENT_PAGE_LIMIT: usize = 256;

/// Session IDs emitted by the authenticated turn path are namespaced as
/// `<principal>:<thread>`. Legacy local-only IDs remain visible only to the
/// local owner; no authenticated principal may claim another namespace.
pub fn session_visible_to(session_id: &SessionId, principal: &PrincipalId) -> bool {
    session_id.0.starts_with(&format!("{}:", principal.0))
        || (principal.0 == LOCAL_OWNER_PRINCIPAL && !session_id.0.contains(':'))
}

pub struct ResumeResult {
    pub session: SessionRecord,
    pub next_sequence: u64,
    pub messages: Vec<::contracts::Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Interrupted,
    AlreadyTerminal,
}

pub struct SessionService {
    store: Arc<dyn SessionAppendStore>,
    active: Arc<ActiveTurnRegistry>,
    interrupted: Mutex<HashSet<String>>,
    protocol: std::sync::Mutex<Connection>,
    runtime_commands: Option<Arc<dyn crate::RuntimeCommandPort>>,
}

impl SessionService {
    async fn ensure_session_visible(
        &self,
        session_id: &SessionId,
        principal: &PrincipalId,
        claim_unowned: bool,
    ) -> Result<()> {
        if let Some(owner) = self.store.principal_for(session_id).await? {
            if owner != *principal {
                bail!("session is not visible to authenticated principal");
            }
            return Ok(());
        }
        if !session_visible_to(session_id, principal) {
            if claim_unowned && self.store.load_session(session_id).await?.is_some() {
                self.store.bind_principal(session_id, principal).await?;
                return Ok(());
            }
            bail!("session is not visible to authenticated principal");
        }
        if claim_unowned {
            self.store.bind_principal(session_id, principal).await?;
        }
        Ok(())
    }

    pub async fn append_protocol_item_event(
        &self,
        session_id: &SessionId,
        item_id: String,
        phase: ::contracts::protocol::client::ItemPhase,
        delta: Option<String>,
        item: Option<ItemRecord>,
        error: Option<String>,
        dedupe_key: Option<String>,
    ) -> Result<::contracts::protocol::client::ClientEvent> {
        let phase_name = format!("{phase:?}").to_ascii_lowercase();
        let mut connection = self.protocol.lock().unwrap_or_else(|e| e.into_inner());
        let tx = connection.transaction()?;
        if let Some(key) = dedupe_key.as_deref() {
            if let Some((sequence, json)) = tx
                .query_row(
                    "SELECT sequence,event_json FROM protocol_events WHERE session_id=?1 AND dedupe_key=?2",
                    params![session_id.0, key],
                    |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
            {
                if item.is_some() {
                    let mut event: ::contracts::protocol::client::ClientEvent = serde_json::from_str(&json)?;
                    if let ::contracts::protocol::client::ClientEvent::Item(existing) = &mut event {
                        existing.item = item.clone();
                        existing.error = error.clone();
                    }
                    tx.execute(
                        "UPDATE protocol_events SET event_json=?3 WHERE session_id=?1 AND sequence=?2",
                        params![session_id.0, sequence, serde_json::to_string(&event)?],
                    )?;
                    tx.commit()?;
                    return Ok(event);
                }
                tx.commit()?;
                return Ok(serde_json::from_str(&json)?);
            }
        }
        let sequence: u64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM protocol_events WHERE session_id=?1",
            params![session_id.0],
            |row| row.get(0),
        )?;
        let cursor = ::contracts::protocol::client::EventCursor {
            sequence,
            event_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let event = ::contracts::protocol::client::ClientEvent::Item(
            ::contracts::protocol::client::ItemEvent {
                cursor: cursor.clone(),
                item_id: item_id.clone(),
                phase,
                delta,
                item,
                error,
            },
        );
        tx.execute(
            "INSERT INTO protocol_events(session_id,sequence,event_id,item_id,phase,dedupe_key,event_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                session_id.0,
                sequence,
                cursor.event_id.as_deref().unwrap_or_default(),
                item_id,
                phase_name,
                dedupe_key,
                serde_json::to_string(&event)?,
            ],
        )?;
        tx.commit()?;
        Ok(event)
    }

    /// Persist a connection-owned approval request in the same authenticated
    /// session event journal used by projection reconnect. The request DTO is
    /// deliberately distinct from the durable ApprovalSnapshot contract: the
    /// opaque choice id is resolved only by the owning connection.
    pub async fn append_protocol_approval_event(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        approval_id: String,
        tool: String,
        action_summary: String,
        risk_level: String,
        detail: Option<String>,
        scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
    ) -> Result<::contracts::protocol::client::ClientEvent> {
        let mut connection = self.protocol.lock().unwrap_or_else(|e| e.into_inner());
        let tx = connection.transaction()?;
        let dedupe_key = format!("approval-request:{approval_id}");
        if let Some(json) = tx
            .query_row(
                "SELECT event_json FROM protocol_events WHERE session_id=?1 AND dedupe_key=?2",
                params![session_id.0, dedupe_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            tx.commit()?;
            return Ok(serde_json::from_str(&json)?);
        }
        let sequence: u64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM protocol_events WHERE session_id=?1",
            params![session_id.0],
            |row| row.get(0),
        )?;
        let cursor = ::contracts::protocol::client::EventCursor {
            sequence,
            event_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let event = ::contracts::protocol::client::ClientEvent::ApprovalRequested {
            cursor: cursor.clone(),
            session_id: session_id.clone(),
            turn_id,
            approval_id: approval_id.clone(),
            tool,
            action_summary,
            risk_level,
            detail,
            scope_subject,
        };
        tx.execute(
            "INSERT INTO protocol_events(session_id,sequence,event_id,item_id,phase,dedupe_key,event_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                session_id.0,
                sequence,
                cursor.event_id.as_deref().unwrap_or_default(),
                approval_id,
                "approval_requested",
                dedupe_key,
                serde_json::to_string(&event)?,
            ],
        )?;
        tx.commit()?;
        Ok(event)
    }

    async fn sync_canonical_protocol_events(&self, session_id: &SessionId) -> Result<()> {
        for item in self.items(session_id).await? {
            let (item_id, phase, error, suffix) = match &item.payload {
                ItemPayload::ToolCall { call_id, .. } => (
                    format!("tool:{}:{call_id}", item.turn_id.0),
                    ::contracts::protocol::client::ItemPhase::Started,
                    None,
                    "tool-started",
                ),
                ItemPayload::ToolResult {
                    call_id,
                    content,
                    is_error,
                    ..
                } => (
                    format!("tool:{}:{call_id}", item.turn_id.0),
                    if *is_error {
                        ::contracts::protocol::client::ItemPhase::Failed
                    } else {
                        ::contracts::protocol::client::ItemPhase::Completed
                    },
                    is_error.then(|| content.clone()),
                    "tool-terminal",
                ),
                ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. } => (
                    format!("turn:{}:assistant", item.turn_id.0),
                    ::contracts::protocol::client::ItemPhase::Completed,
                    None,
                    "assistant-terminal",
                ),
                _ => (
                    item.id.0.to_string(),
                    ::contracts::protocol::client::ItemPhase::Completed,
                    None,
                    "canonical-terminal",
                ),
            };
            self.append_protocol_item_event(
                session_id,
                item_id,
                phase,
                None,
                Some(item.clone()),
                error,
                Some(format!(
                    "{}:{suffix}",
                    match &item.payload {
                        ItemPayload::ToolCall { call_id, .. }
                        | ItemPayload::ToolResult { call_id, .. } =>
                            format!("tool:{}:{call_id}", item.turn_id.0),
                        ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. } =>
                            format!("turn:{}:assistant", item.turn_id.0),
                        _ => item.id.0.to_string(),
                    }
                )),
            )
            .await?;
        }
        Ok(())
    }
    pub fn new(store: Arc<dyn SessionAppendStore>, active: Arc<ActiveTurnRegistry>) -> Self {
        Self::with_protocol_journal(store, active, ":memory:").expect("in-memory protocol journal")
    }

    pub fn with_protocol_journal(
        store: Arc<dyn SessionAppendStore>,
        active: Arc<ActiveTurnRegistry>,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS protocol_events(
               session_id TEXT NOT NULL,
               sequence INTEGER NOT NULL,
               event_id TEXT NOT NULL,
               item_id TEXT NOT NULL,
               phase TEXT NOT NULL,
               dedupe_key TEXT,
               event_json TEXT NOT NULL,
               PRIMARY KEY(session_id,sequence),
               UNIQUE(session_id,event_id),
               UNIQUE(session_id,dedupe_key)
             );",
        )?;
        Ok(Self {
            store,
            active,
            interrupted: Mutex::new(HashSet::new()),
            protocol: std::sync::Mutex::new(connection),
            runtime_commands: None,
        })
    }

    /// Bind the one-way Runtime command owner used by production composition.
    /// Read-only fixtures may leave this unset and exercise direct store APIs.
    pub fn with_runtime_commands(mut self, commands: Arc<dyn crate::RuntimeCommandPort>) -> Self {
        self.runtime_commands = Some(commands);
        self
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
    ) -> Result<::contracts::protocol::client::UiSnapshot> {
        self.protocol_snapshot_for(&PrincipalId(LOCAL_OWNER_PRINCIPAL.into()), session_id)
            .await
    }

    pub async fn protocol_snapshot_for(
        &self,
        principal: &PrincipalId,
        session_id: &SessionId,
    ) -> Result<::contracts::protocol::client::UiSnapshot> {
        let snapshot = self
            .protocol_read_snapshot_for(principal, session_id)
            .await?;
        Ok(::contracts::protocol::client::UiSnapshot {
            session_id: session_id.clone(),
            cursor: snapshot.through,
            provider: None,
            model: None,
            items: snapshot.items,
            approvals: Vec::new(),
            agents: Vec::new(),
        })
    }

    pub async fn protocol_read_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<::contracts::protocol::client::SessionReadSnapshot> {
        self.protocol_read_snapshot_for(&PrincipalId(LOCAL_OWNER_PRINCIPAL.into()), session_id)
            .await
    }

    pub async fn protocol_read_snapshot_for(
        &self,
        principal: &PrincipalId,
        session_id: &SessionId,
    ) -> Result<::contracts::protocol::client::SessionReadSnapshot> {
        self.ensure_session_visible(session_id, principal, true)
            .await?;
        let session = self
            .store
            .load_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session not found"))?;
        let items = self.items(session_id).await?;
        self.sync_canonical_protocol_events(session_id).await?;
        let cursor = self.protocol_tail_cursor(session_id)?;
        let (tasks, activities) = SessionProjection::read_model(&session, &items);
        Ok(::contracts::protocol::client::SessionReadSnapshot {
            schema_version: ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION,
            session,
            through: cursor,
            items,
            tasks,
            activities,
        })
    }

    pub async fn protocol_session_list(
        &self,
    ) -> Result<::contracts::protocol::client::SessionListSnapshot> {
        self.protocol_session_list_for(&PrincipalId(LOCAL_OWNER_PRINCIPAL.into()))
            .await
    }

    pub async fn protocol_session_list_for(
        &self,
        principal: &PrincipalId,
    ) -> Result<::contracts::protocol::client::SessionListSnapshot> {
        let mut sessions = Vec::new();
        for session in self.store.list_sessions(256).await? {
            let visible = match self.store.principal_for(&session.id).await? {
                Some(owner) => owner == *principal,
                None => session_visible_to(&session.id, principal),
            };
            if visible {
                sessions.push(session);
            }
        }
        Ok(::contracts::protocol::client::SessionListSnapshot {
            schema_version: ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION,
            sessions,
        })
    }

    /// Replay durable item terminals strictly after an authenticated cursor.
    /// A non-origin cursor must name the item at its sequence; this prevents a
    /// stale or forged `(sequence,event_id)` pair from skipping history.
    pub async fn protocol_events_after(
        &self,
        session_id: &SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> Result<Vec<::contracts::protocol::client::ClientEvent>> {
        self.protocol_events_after_for(
            &PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
            session_id,
            after,
        )
        .await
    }

    pub async fn protocol_events_after_for(
        &self,
        principal: &PrincipalId,
        session_id: &SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> Result<Vec<::contracts::protocol::client::ClientEvent>> {
        Ok(self
            .protocol_event_page_for(principal, session_id, after)
            .await?
            .events)
    }

    pub async fn protocol_event_page(
        &self,
        session_id: &SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> Result<::contracts::protocol::client::SessionEventPage> {
        self.protocol_event_page_for(
            &PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
            session_id,
            after,
        )
        .await
    }

    pub async fn protocol_event_page_for(
        &self,
        principal: &PrincipalId,
        session_id: &SessionId,
        after: &::contracts::protocol::client::EventCursor,
    ) -> Result<::contracts::protocol::client::SessionEventPage> {
        self.ensure_session_visible(session_id, principal, true)
            .await?;
        self.sync_canonical_protocol_events(session_id).await?;
        if after.sequence == 0 {
            if after.event_id.is_some() {
                bail!("origin cursor cannot carry an event_id");
            }
        } else {
            let anchor_event_id: Option<String> = self
                .protocol
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .query_row(
                    "SELECT event_id FROM protocol_events WHERE session_id=?1 AND sequence=?2",
                    params![session_id.0, after.sequence],
                    |row| row.get(0),
                )
                .optional()?;
            if anchor_event_id.as_deref() != after.event_id.as_deref() {
                bail!("cursor event_id does not match durable item");
            }
        }
        let connection = self.protocol.lock().unwrap_or_else(|e| e.into_inner());
        let mut statement = connection.prepare(
            "SELECT event_json FROM protocol_events WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        )?;
        let rows = statement
            .query_map(
                params![session_id.0, after.sequence, SESSION_EVENT_PAGE_LIMIT],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let events = rows
            .into_iter()
            .map(|json| serde_json::from_str(&json))
            .collect::<serde_json::Result<Vec<::contracts::protocol::client::ClientEvent>>>()?;
        let next = events.last().map_or_else(
            || after.clone(),
            |event| match event {
                ::contracts::protocol::client::ClientEvent::Item(item) => item.cursor.clone(),
                ::contracts::protocol::client::ClientEvent::ApprovalRequested {
                    cursor, ..
                } => cursor.clone(),
                _ => after.clone(),
            },
        );
        Ok(::contracts::protocol::client::SessionEventPage {
            schema_version: ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION,
            session_id: session_id.clone(),
            after: after.clone(),
            next,
            events,
        })
    }

    fn protocol_tail_cursor(
        &self,
        session_id: &SessionId,
    ) -> Result<::contracts::protocol::client::EventCursor> {
        let row: Option<(u64, String)> = self.protocol.lock().unwrap_or_else(|e| e.into_inner()).query_row(
            "SELECT sequence,event_id FROM protocol_events WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1",
            params![session_id.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        Ok(row.map_or_else(
            ::contracts::protocol::client::EventCursor::origin,
            |(sequence, event_id)| ::contracts::protocol::client::EventCursor {
                sequence,
                event_id: Some(event_id),
            },
        ))
    }

    /// Persist lifecycle-provided workspace context into canonical history
    /// before it is used for model projection. The Fabric validator is the
    /// single authority for effect bounds and phase legality.
    pub async fn persist_context_fragments(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        phase: crate::lifecycle::LifecyclePhase,
        fragments: Vec<(String, String)>,
    ) -> Result<usize> {
        if fragments.is_empty() {
            return Ok(0);
        }
        let effects = fragments
            .into_iter()
            .map(
                |(source, content)| crate::lifecycle::LifecycleEffect::AddContextFragment {
                    source,
                    content,
                },
            )
            .collect::<Vec<_>>();
        let items = self.items(session_id).await?;
        let mut sequence = items.last().map_or(1, |item| item.sequence + 1);
        crate::context_fragment::inject_context_fragments(
            self.store.as_ref(),
            session_id,
            turn_id,
            &mut sequence,
            phase,
            &effects,
        )
        .await
    }

    /// Persist the authoritative per-turn budget and any preflight compaction
    /// evidence before inference starts, so a concurrent status read observes
    /// the same snapshot used by context assembly.
    pub async fn persist_turn_start_budget(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        projection: ::contracts::ContextBudgetProjection,
        compactions: Vec<::contracts::ContextCompactionProjection>,
    ) -> Result<usize> {
        let items = self.items(session_id).await?;
        let existing_budgets = items
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::ContextBudgetProjection { projection } if item.turn_id == turn_id => {
                    Some(projection.as_ref())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            existing_budgets.len() <= 1,
            "turn has duplicate context budget projections"
        );
        if let Some(existing) = existing_budgets.first() {
            anyhow::ensure!(
                *existing == &projection,
                "turn already has a different context budget projection"
            );
        }
        let existing_compactions = items
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::ContextCompactionProjection { projection }
                    if item.turn_id == turn_id =>
                {
                    Some(projection.as_ref())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            existing_compactions.len() <= compactions.len(),
            "turn has more context compaction projections than the retry"
        );
        anyhow::ensure!(
            existing_compactions
                .iter()
                .zip(&compactions)
                .all(|(existing, expected)| *existing == expected),
            "turn already has different context compaction projections"
        );
        anyhow::ensure!(
            !existing_budgets.is_empty() || existing_compactions.is_empty(),
            "turn has context compaction evidence without its budget projection"
        );

        let mut sequence = items.last().map_or(1, |item| item.sequence + 1);
        let mut created_at_ms = items
            .last()
            .map_or(0, |item| item.created_at_ms.saturating_add(1));
        let mut payloads = Vec::with_capacity(1 + compactions.len());
        if existing_budgets.is_empty() {
            payloads.push(ItemPayload::ContextBudgetProjection {
                projection: Box::new(projection),
            });
        }
        payloads.extend(
            compactions
                .into_iter()
                .skip(existing_compactions.len())
                .map(|projection| ItemPayload::ContextCompactionProjection {
                    projection: Box::new(projection),
                }),
        );
        let count = payloads.len();
        for payload in payloads {
            self.store
                .append(
                    session_id,
                    sequence,
                    ItemRecord {
                        schema_version: SESSION_SCHEMA_VERSION,
                        id: ItemId::new(),
                        session_id: session_id.clone(),
                        turn_id,
                        sequence,
                        created_at_ms,
                        payload,
                    },
                )
                .await?;
            sequence = sequence.saturating_add(1);
            created_at_ms = created_at_ms.saturating_add(1);
        }
        Ok(count)
    }

    /// Persist Host-authored Task projection inputs in the canonical Session
    /// history. The item participates in the same expected-sequence and
    /// idempotency rules as every other durable Session fact.
    pub async fn persist_task_projection_fact(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        item_id: ItemId,
        fact: TaskProjectionFact,
    ) -> Result<AppendOutcome> {
        if fact.schema_version != ::contracts::TASK_PROJECTION_FACT_SCHEMA_VERSION {
            bail!(
                "unsupported task projection fact schema version {}",
                fact.schema_version
            );
        }
        let items = self.items(session_id).await?;
        let sequence = items.last().map_or(1, |item| item.sequence + 1);
        let created_at_ms = items
            .last()
            .map_or(0, |item| item.created_at_ms.saturating_add(1));
        self.store
            .append(
                session_id,
                sequence,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: item_id,
                    session_id: session_id.clone(),
                    turn_id,
                    sequence,
                    created_at_ms,
                    payload: ItemPayload::TaskProjection { fact },
                },
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
        if let Some(commands) = &self.runtime_commands {
            let receipt = commands
                .dispatch(crate::RuntimeCommand::ForkSession(
                    crate::ForkSessionCommand {
                        session: crate::SessionId(parent.0.clone()),
                        through_sequence,
                    },
                ))
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let child = receipt
                .session
                .ok_or_else(|| anyhow::anyhow!("Runtime fork receipt omitted session id"))?;
            let child = SessionId(child.0);
            return self
                .store
                .load_session(&child)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Runtime fork did not materialize child session"));
        }
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

    /// Resolve the canonical event boundary for a turn in a session.
    ///
    /// Checkpoint clients must not guess that a user-facing prompt index is an
    /// event sequence. A turn can contain multiple items, so a historical fork
    /// is anchored after the last persisted item owned by that turn.
    pub async fn sequence_through_turn(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
    ) -> Result<u64> {
        self.store
            .load_items(session_id, None)
            .await?
            .into_iter()
            .filter(|item| item.turn_id == turn_id)
            .map(|item| item.sequence)
            .max()
            .ok_or_else(|| anyhow::anyhow!("checkpoint turn is absent from session authority"))
    }

    pub async fn replay(
        &self,
        session_id: &SessionId,
        after: Option<u64>,
    ) -> Result<Vec<::contracts::Message>> {
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
        active.cancel(::contracts::CancelReason::User);
        interrupted.insert(session_id.0.clone());
        Ok(InterruptOutcome::Interrupted)
    }
}

fn legacy_message_payloads(message: &Message) -> Vec<ItemPayload> {
    let mut payloads = Vec::new();
    for block in &message.content {
        let payload = match block {
            ContentBlock::Text { text } => match message.role {
                Role::User => ItemPayload::UserMessage {
                    content: text.clone(),
                    execution_target: ::contracts::ExecutionTargetSelection::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Behaviour-complete in-memory store: appended items and principal
    /// bindings are durable for the lifetime of the test, matching the
    /// event-sourced store used by the legacy Executive service.
    #[derive(Default)]
    struct TestMemoryStore {
        sessions: std::sync::Mutex<Vec<::contracts::SessionRecord>>,
        items: std::sync::Mutex<Vec<::contracts::ItemRecord>>,
        principals: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl SessionAppendStore for TestMemoryStore {
        async fn create(&self, session: ::contracts::SessionRecord) -> anyhow::Result<()> {
            self.sessions.lock().unwrap().push(session);
            Ok(())
        }
        async fn append(
            &self,
            _session: &::contracts::SessionId,
            _expected_sequence: u64,
            item: ::contracts::ItemRecord,
        ) -> anyhow::Result<::contracts::AppendOutcome> {
            self.items.lock().unwrap().push(item);
            Ok(::contracts::AppendOutcome::Appended)
        }
        async fn fork(
            &self,
            _parent: &::contracts::SessionId,
            _through_sequence: u64,
            child: ::contracts::SessionRecord,
        ) -> anyhow::Result<()> {
            self.sessions.lock().unwrap().push(child);
            Ok(())
        }
        async fn bind_principal(
            &self,
            session: &::contracts::SessionId,
            principal: &::contracts::PrincipalId,
        ) -> anyhow::Result<()> {
            self.principals
                .lock()
                .unwrap()
                .push((session.0.clone(), principal.0.clone()));
            Ok(())
        }
    }

    #[async_trait]
    impl ::contracts::SessionReadStore for TestMemoryStore {
        async fn load_session(
            &self,
            session: &::contracts::SessionId,
        ) -> anyhow::Result<Option<::contracts::SessionRecord>> {
            Ok(self
                .sessions
                .lock()
                .unwrap()
                .iter()
                .find(|r| &r.id == session)
                .cloned())
        }
        async fn load_items(
            &self,
            session: &::contracts::SessionId,
            _after: Option<u64>,
        ) -> anyhow::Result<Vec<::contracts::ItemRecord>> {
            Ok(self
                .items
                .lock()
                .unwrap()
                .iter()
                .filter(|i| &i.session_id == session)
                .cloned()
                .collect())
        }
        async fn list_sessions(
            &self,
            _limit: usize,
        ) -> anyhow::Result<Vec<::contracts::SessionRecord>> {
            Ok(self.sessions.lock().unwrap().clone())
        }
        async fn principal_for(
            &self,
            session: &::contracts::SessionId,
        ) -> anyhow::Result<Option<::contracts::PrincipalId>> {
            Ok(self
                .principals
                .lock()
                .unwrap()
                .iter()
                .find(|(sid, _)| sid == &session.0)
                .map(|(_, p)| ::contracts::PrincipalId(p.clone())))
        }
    }

    fn test_store() -> Arc<dyn SessionAppendStore> {
        Arc::new(TestMemoryStore::default())
    }

    fn test_budget_projection() -> ::contracts::ContextBudgetProjection {
        let agent_source = ::contracts::ContextBudgetSource::new(
            ::contracts::ContextBudgetSourceKind::AgentRuntime,
            "current Agent rollout scope",
        );
        ::contracts::ContextBudgetProjection {
            model_spec: "deepseek/deepseek-v4-flash[1m]".into(),
            model_context_tokens: 1_000_000.into(),
            profile_input_limit_tokens: 1_000_000.into(),
            reserved_output_tokens: 16_384.into(),
            system_and_skill_tokens: 10_000.into(),
            tool_schema_tokens: 8_000.into(),
            pending_input_tokens: 1_000.into(),
            safety_margin_tokens: 50_000.into(),
            current_history_tokens: 83_000.into(),
            admissible_history_tokens: 915_616.into(),
            compaction_threshold_tokens: 801_164.into(),
            model_source: ::contracts::ContextBudgetSource::new(
                ::contracts::ContextBudgetSourceKind::RuntimeModelCapability,
                "deepseek/deepseek-v4-flash[1m]",
            ),
            profile_source: ::contracts::ContextBudgetSource::new(
                ::contracts::ContextBudgetSourceKind::ActiveAgentProfile,
                "general",
            ),
            history_source: ::contracts::ContextBudgetSource::new(
                ::contracts::ContextBudgetSourceKind::ContextBudgetPlanner,
                "ContextBudgetPlanner",
            ),
            rollout: ::contracts::RolloutBudgetProjection {
                root_remaining_tokens: ::contracts::RolloutBudgetValue::Unknown {
                    source: agent_source.clone(),
                    reason: ::contracts::BudgetMissingReason::NoActiveAgentRollout,
                },
                child_limit_tokens: ::contracts::RolloutBudgetValue::Known {
                    value: 200_000.into(),
                    source: ::contracts::ContextBudgetSource::new(
                        ::contracts::ContextBudgetSourceKind::EffectiveAdmissionConfig,
                        "agent.admission.max_child_tokens",
                    ),
                },
                current_agent_remaining_tokens: ::contracts::RolloutBudgetValue::Unknown {
                    source: agent_source,
                    reason: ::contracts::BudgetMissingReason::NoActiveAgentRollout,
                },
            },
        }
    }

    #[tokio::test]
    async fn turn_start_budget_is_durable_visible_and_idempotent_before_terminal_items() {
        let store = test_store();
        let session_id = SessionId("turn-start-budget".into());
        let turn_id = TurnId::new();
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
        store
            .append(
                &session_id,
                1,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id,
                    sequence: 1,
                    created_at_ms: 1,
                    payload: ItemPayload::UserMessage {
                        content: "inspect".into(),
                        execution_target: ::contracts::ExecutionTargetSelection::default(),
                    },
                },
            )
            .await
            .unwrap();
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));
        let projection = test_budget_projection();
        let compaction = ::contracts::ContextCompactionProjection {
            mode: ::contracts::ContextCompactionMode::Preflight,
            trigger_threshold_tokens: projection.admissible_history_tokens,
            tokens_before: 930_000.into(),
            tokens_after: 80_000.into(),
            budget_snapshot: projection.clone(),
        };

        assert_eq!(
            service
                .persist_turn_start_budget(
                    &session_id,
                    turn_id,
                    projection.clone(),
                    vec![compaction.clone()],
                )
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            service
                .persist_turn_start_budget(&session_id, turn_id, projection, vec![compaction],)
                .await
                .unwrap(),
            0
        );
        let snapshot = service.protocol_read_snapshot(&session_id).await.unwrap();
        assert_eq!(snapshot.items.len(), 3);
        let budget = snapshot.tasks[0]
            .runtime_facts
            .as_ref()
            .and_then(|facts| facts.context_budget.as_deref())
            .expect("turn-start status budget");
        assert_eq!(budget.model_context_tokens.get(), 1_000_000);
    }

    #[tokio::test]
    async fn turn_start_budget_retry_completes_a_partially_persisted_compaction_prefix() {
        let store = test_store();
        let session_id = SessionId("turn-start-budget-partial".into());
        let turn_id = TurnId::new();
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
        let projection = test_budget_projection();
        store
            .append(
                &session_id,
                1,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id,
                    sequence: 1,
                    created_at_ms: 1,
                    payload: ItemPayload::ContextBudgetProjection {
                        projection: Box::new(projection.clone()),
                    },
                },
            )
            .await
            .unwrap();
        let compaction = ::contracts::ContextCompactionProjection {
            mode: ::contracts::ContextCompactionMode::Preflight,
            trigger_threshold_tokens: projection.admissible_history_tokens,
            tokens_before: 930_000.into(),
            tokens_after: 80_000.into(),
            budget_snapshot: projection.clone(),
        };
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));

        assert_eq!(
            service
                .persist_turn_start_budget(
                    &session_id,
                    turn_id,
                    projection,
                    vec![compaction.clone()],
                )
                .await
                .unwrap(),
            1
        );
        let items = service.items(&session_id).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1].payload,
            ItemPayload::ContextCompactionProjection { projection }
                if projection.as_ref() == &compaction
        ));
    }

    #[test]
    fn session_visibility_is_principal_scoped_and_legacy_local_only() {
        let owner = PrincipalId("local-uid:1000".into());
        let other = PrincipalId("local-uid:2000".into());
        assert!(session_visible_to(
            &SessionId("local-uid:1000:thread-a".into()),
            &owner
        ));
        assert!(!session_visible_to(
            &SessionId("local-uid:1000:thread-a".into()),
            &other
        ));
        assert!(session_visible_to(
            &SessionId("legacy-session".into()),
            &PrincipalId(LOCAL_OWNER_PRINCIPAL.into())
        ));
        assert!(!session_visible_to(
            &SessionId("legacy-session".into()),
            &other
        ));
    }

    #[tokio::test]
    async fn lifecycle_context_fragment_is_bounded_and_durable() {
        let store = test_store();
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
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));
        let persisted = service
            .persist_context_fragments(
                &session_id,
                TurnId::new(),
                crate::lifecycle::LifecyclePhase::BeforeTurnInput,
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

    #[tokio::test]
    async fn u_resume_006_protocol_picker_and_snapshot_enforce_durable_principal_ownership() {
        let store = test_store();
        let first = SessionId("shared-thread".into());
        let second = SessionId("other-thread".into());
        for id in [&first, &second] {
            store
                .create(SessionRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: id.clone(),
                    parent: None,
                    created_at_ms: 1,
                    status: SessionStatus::Active,
                })
                .await
                .unwrap();
        }
        let owner = PrincipalId("local-uid:1000".into());
        let other = PrincipalId("local-uid:2000".into());
        store.bind_principal(&first, &owner).await.unwrap();
        store.bind_principal(&second, &other).await.unwrap();
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));

        let list = service.protocol_session_list_for(&owner).await.unwrap();
        assert_eq!(
            list.sessions.iter().map(|s| &s.id).collect::<Vec<_>>(),
            vec![&first]
        );
        let denied = service
            .protocol_read_snapshot_for(&other, &first)
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("not visible"));
    }

    #[tokio::test]
    async fn historical_fork_boundary_uses_last_item_in_checkpoint_turn() {
        let store = test_store();
        let session_id = SessionId("fork-boundary-session".into());
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
        let checkpoint_turn = TurnId::new();
        for sequence in [1, 2] {
            store
                .append(
                    &session_id,
                    sequence,
                    ItemRecord {
                        schema_version: SESSION_SCHEMA_VERSION,
                        id: ItemId::new(),
                        session_id: session_id.clone(),
                        turn_id: checkpoint_turn,
                        sequence,
                        created_at_ms: sequence,
                        payload: ItemPayload::SystemNotice {
                            content: format!("item-{sequence}"),
                        },
                    },
                )
                .await
                .unwrap();
        }
        store
            .append(
                &session_id,
                3,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id: TurnId::new(),
                    sequence: 3,
                    created_at_ms: 3,
                    payload: ItemPayload::SystemNotice {
                        content: "later".into(),
                    },
                },
            )
            .await
            .unwrap();
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));

        assert_eq!(
            service
                .sequence_through_turn(&session_id, checkpoint_turn)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn approval_request_is_durable_and_replayed_after_cursor() {
        let store = test_store();
        let session_id = SessionId("approval-replay-session".into());
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
        let service = SessionService::new(store, Arc::new(ActiveTurnRegistry::new()));
        let event = service
            .append_protocol_approval_event(
                &session_id,
                TurnId::new(),
                "choice-1".into(),
                "bash_exec".into(),
                "remove temporary file".into(),
                "high".into(),
                Some("rm /tmp/example".into()),
                None,
            )
            .await
            .unwrap();
        let cursor = match event {
            ::contracts::protocol::client::ClientEvent::ApprovalRequested {
                approval_id,
                cursor,
                ..
            } => {
                assert_eq!(approval_id, "choice-1");
                cursor
            }
            other => panic!("unexpected protocol event: {other:?}"),
        };
        let page = service
            .protocol_event_page_for(
                &PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
                &session_id,
                &::contracts::protocol::client::EventCursor::origin(),
            )
            .await
            .unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.next, cursor);
        let empty = service
            .protocol_event_page_for(
                &PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
                &session_id,
                &cursor,
            )
            .await
            .unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.next, cursor);
    }
}
