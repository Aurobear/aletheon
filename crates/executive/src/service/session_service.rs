//! Resume, fork, interrupt, and replay over canonical session history.

use std::{collections::HashSet, sync::Arc};

use anyhow::{bail, Result};
use fabric::{
    SessionAppendStore, SessionFork, SessionId, SessionRecord, SessionStatus,
    SESSION_SCHEMA_VERSION,
};
use tokio::sync::Mutex;

use crate::r#impl::session::canonical_store::project_messages;

use super::turn_coordinator::ActiveTurn;

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
    active: Arc<Mutex<std::collections::HashMap<String, ActiveTurn>>>,
    interrupted: Mutex<HashSet<String>>,
}

impl SessionService {
    pub fn new(
        store: Arc<dyn SessionAppendStore>,
        active: Arc<Mutex<std::collections::HashMap<String, ActiveTurn>>>,
    ) -> Self {
        Self {
            store,
            active,
            interrupted: Mutex::new(HashSet::new()),
        }
    }

    pub async fn resume(&self, session_id: &SessionId) -> Result<ResumeResult> {
        let session = self
            .store
            .load_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("session not found"))?;
        let items = self.store.load_items(session_id, None).await?;
        let next_sequence = items.last().map_or(1, |item| item.sequence + 1);
        Ok(ResumeResult {
            session,
            next_sequence,
            messages: project_messages(&items)?,
        })
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
        let active = self.active.lock().await.get(&session_id.0).cloned();
        let Some(active) = active else {
            return Ok(InterruptOutcome::AlreadyTerminal);
        };
        active.cancel.cancel();
        interrupted.insert(session_id.0.clone());
        Ok(InterruptOutcome::Interrupted)
    }
}
