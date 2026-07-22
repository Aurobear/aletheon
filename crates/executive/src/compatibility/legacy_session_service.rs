//! Request-safe legacy session compatibility use cases.

use crate::host::daemon::session_manager::SessionManager;
use crate::adapters::session::store::SessionStore;
use async_trait::async_trait;
use fabric::{Clock, ContentBlock, LlmProvider, Message, Role, SessionId};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::application::session_service::SessionService;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LegacySessionView {
    pub session_id: String,
    pub message_count: usize,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct LegacySessionSnapshot {
    pub session_id: String,
    pub turn_count: usize,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug)]
pub struct LegacySessionTransition {
    pub previous: LegacySessionSnapshot,
    pub current: LegacySessionView,
}

#[derive(Debug, Error)]
pub enum LegacySessionError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("session operation failed: {0}")]
    Operation(String),
}

#[async_trait]
pub trait LegacySessionUseCases: Send + Sync {
    async fn create(&self) -> Result<LegacySessionView, LegacySessionError>;
    async fn create_and_switch(
        &self,
        previous_thread_id: &str,
    ) -> Result<LegacySessionTransition, LegacySessionError>;
    async fn list(&self) -> Result<Vec<LegacySessionView>, LegacySessionError>;
    async fn list_available(&self) -> Result<Vec<String>, LegacySessionError>;
    async fn switch(&self, session_id: String) -> Result<String, LegacySessionError>;
    async fn resume(&self, session_id: String)
        -> Result<LegacySessionSnapshot, LegacySessionError>;
    async fn load_recent(&self) -> Result<LegacySessionSnapshot, LegacySessionError>;
    async fn clear(&self, thread_id: &str) -> Result<LegacySessionTransition, LegacySessionError>;
    async fn compact(
        &self,
        thread_id: &str,
    ) -> Result<Option<LegacySessionTransition>, LegacySessionError>;
    async fn current(&self, thread_id: &str) -> Result<LegacySessionSnapshot, LegacySessionError>;
    async fn route_workspace(&self, working_dir: PathBuf) -> Result<String, LegacySessionError>;
}

pub struct LegacySessionService {
    registry: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    workspace_sessions: Mutex<HashMap<PathBuf, String>>,
    data_dir: PathBuf,
    context_window: usize,
    clock: Arc<dyn Clock>,
    llm: Arc<dyn LlmProvider>,
    canonical: Arc<SessionService>,
}

pub struct LegacySessionResources {
    pub registry: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    pub created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    pub data_dir: PathBuf,
    pub context_window: usize,
    pub clock: Arc<dyn Clock>,
    pub llm: Arc<dyn LlmProvider>,
    pub canonical: Arc<SessionService>,
}

// Note: default_id has been removed from LegacySessionResources.
// Callers must pass explicit session_id to all trait methods.

impl LegacySessionService {
    pub fn new(resources: LegacySessionResources) -> Self {
        Self {
            registry: resources.registry,
            created_at: resources.created_at,
            workspace_sessions: Mutex::new(HashMap::new()),
            data_dir: resources.data_dir,
            context_window: resources.context_window,
            clock: resources.clock,
            llm: resources.llm,
            canonical: resources.canonical,
        }
    }

    async fn manager(
        &self,
        session_id: &str,
    ) -> Result<Arc<Mutex<SessionManager>>, LegacySessionError> {
        if let Some(manager) = self.registry.lock().await.get(session_id).cloned() {
            return Ok(manager);
        }
        let manager = SessionManager::new(
            &self.data_dir,
            session_id.to_owned(),
            self.context_window,
            self.clock.clone(),
        )
        .await
        .map_err(operation_error)?;
        let mut manager = manager;
        if let Some(replay) = self
            .canonical
            .try_resume(&SessionId(session_id.to_owned()))
            .await
            .map_err(operation_error)?
        {
            manager.restore_messages(replay.messages);
        }
        let manager = Arc::new(Mutex::new(manager));
        self.registry
            .lock()
            .await
            .insert(session_id.to_owned(), manager.clone());
        self.created_at
            .lock()
            .await
            .entry(session_id.to_owned())
            .or_insert_with(|| self.clock.mono_now());
        Ok(manager)
    }

    async fn snapshot(
        &self,
        session_id: &str,
    ) -> Result<LegacySessionSnapshot, LegacySessionError> {
        let manager = self.manager(session_id).await?;
        let manager = manager.lock().await;
        Ok(LegacySessionSnapshot {
            session_id: manager.session_id.clone(),
            turn_count: manager.turn_count(),
            messages: manager.history().to_vec(),
        })
    }

    async fn project(&self, snapshot: &LegacySessionSnapshot) -> Result<(), LegacySessionError> {
        self.canonical
            .ensure_legacy_projection(
                &SessionId(snapshot.session_id.clone()),
                &snapshot.messages,
                fabric::wall_to_datetime(self.clock.wall_now())
                    .timestamp_millis()
                    .max(0) as u64,
            )
            .await
            .map_err(operation_error)
    }

    async fn create_with_messages(
        &self,
        messages: &[Message],
    ) -> Result<LegacySessionView, LegacySessionError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        SessionStore::new(&self.data_dir)
            .and_then(|store| store.create_session(&session_id))
            .map_err(operation_error)?;
        let mut manager = SessionManager::new(
            &self.data_dir,
            session_id.clone(),
            self.context_window,
            self.clock.clone(),
        )
        .await
        .map_err(operation_error)?;
        for message in messages {
            persist_legacy_message(&mut manager, message).await;
        }
        let snapshot = LegacySessionSnapshot {
            session_id: session_id.clone(),
            turn_count: manager.turn_count(),
            messages: manager.history().to_vec(),
        };
        self.registry
            .lock()
            .await
            .insert(session_id.clone(), Arc::new(Mutex::new(manager)));
        self.created_at
            .lock()
            .await
            .insert(session_id.clone(), self.clock.mono_now());
        self.project(&snapshot).await?;
        Ok(LegacySessionView {
            session_id,
            message_count: snapshot.messages.len(),
            created_at: fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
        })
    }

    async fn remap_default_workspace(&self, previous: &str, current: &str) {
        for session_id in self.workspace_sessions.lock().await.values_mut() {
            if session_id == previous {
                *session_id = current.to_owned();
            }
        }
    }
}

#[async_trait]
impl LegacySessionUseCases for LegacySessionService {
    async fn create(&self) -> Result<LegacySessionView, LegacySessionError> {
        self.create_with_messages(&[]).await
    }

    async fn create_and_switch(
        &self,
        previous_thread_id: &str,
    ) -> Result<LegacySessionTransition, LegacySessionError> {
        let previous = self.current(previous_thread_id).await?;
        let current = self.create_with_messages(&[]).await?;
        self.remap_default_workspace(&previous.session_id, &current.session_id)
            .await;
        Ok(LegacySessionTransition { previous, current })
    }

    async fn list(&self) -> Result<Vec<LegacySessionView>, LegacySessionError> {
        let entries: Vec<_> = self
            .registry
            .lock()
            .await
            .iter()
            .take(100)
            .map(|(id, manager)| (id.clone(), manager.clone()))
            .collect();
        let created = self.created_at.lock().await.clone();
        let now = self.clock.mono_now().0;
        let mut result = Vec::with_capacity(entries.len());
        for (session_id, manager) in entries {
            let message_count = manager.lock().await.message_count();
            let created_at = created
                .get(&session_id)
                .map(|time| format!("{}s ago", now.saturating_sub(time.0) / 1000))
                .unwrap_or_else(|| "unknown".into());
            result.push(LegacySessionView {
                session_id,
                message_count,
                created_at,
            });
        }
        result.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        Ok(result)
    }

    async fn list_available(&self) -> Result<Vec<String>, LegacySessionError> {
        SessionStore::new(&self.data_dir)
            .and_then(|store| store.list_sessions())
            .map(|ids| ids.into_iter().take(100).collect())
            .map_err(operation_error)
    }

    async fn switch(&self, session_id: String) -> Result<String, LegacySessionError> {
        let available = SessionStore::new(&self.data_dir)
            .and_then(|store| store.load(&session_id))
            .map_err(operation_error)?
            .is_some();
        if !available && !self.registry.lock().await.contains_key(&session_id) {
            return Err(LegacySessionError::NotFound(session_id));
        }
        let snapshot = self.snapshot(&session_id).await?;
        self.project(&snapshot).await?;
        Ok(session_id)
    }

    async fn resume(
        &self,
        session_id: String,
    ) -> Result<LegacySessionSnapshot, LegacySessionError> {
        let canonical = self
            .canonical
            .try_resume(&SessionId(session_id.clone()))
            .await
            .map_err(operation_error)?;
        if canonical.is_none()
            && SessionStore::new(&self.data_dir)
                .and_then(|store| store.load(&session_id))
                .map_err(operation_error)?
                .is_none()
        {
            return Err(LegacySessionError::NotFound(session_id));
        }
        if let Some(replay) = canonical {
            self.manager(&session_id)
                .await?
                .lock()
                .await
                .restore_messages(replay.messages);
        }
        let snapshot = self.snapshot(&session_id).await?;
        self.project(&snapshot).await?;
        Ok(snapshot)
    }

    async fn load_recent(&self) -> Result<LegacySessionSnapshot, LegacySessionError> {
        let recent = SessionStore::new(&self.data_dir)
            .and_then(|store| store.most_recent())
            .map_err(operation_error)?;
        match recent {
            Some(session_id) => self.resume(session_id).await,
            None => {
                let current = self.create_with_messages(&[]).await?;
                self.snapshot(&current.session_id).await
            }
        }
    }

    async fn clear(&self, thread_id: &str) -> Result<LegacySessionTransition, LegacySessionError> {
        let previous = self.current(thread_id).await?;
        self.manager(&previous.session_id)
            .await?
            .lock()
            .await
            .clear_history()
            .await
            .map_err(operation_error)?;
        let current = self.create_with_messages(&[]).await?;
        self.remap_default_workspace(&previous.session_id, &current.session_id)
            .await;
        Ok(LegacySessionTransition { previous, current })
    }

    async fn compact(
        &self,
        thread_id: &str,
    ) -> Result<Option<LegacySessionTransition>, LegacySessionError> {
        let previous = self.current(thread_id).await?;
        let manager = self.manager(&previous.session_id).await?;
        let messages = {
            let mut manager = manager.lock().await;
            if !manager
                .force_compact(&*self.llm)
                .await
                .map_err(operation_error)?
            {
                return Ok(None);
            }
            manager.history().to_vec()
        };
        // Canonical history is immutable. Materialize the compacted view as a
        // new session rather than silently rewriting durable Session/Turn/Item truth.
        let current = self.create_with_messages(&messages).await?;
        self.remap_default_workspace(&previous.session_id, &current.session_id)
            .await;
        Ok(Some(LegacySessionTransition { previous, current }))
    }

    async fn current(&self, thread_id: &str) -> Result<LegacySessionSnapshot, LegacySessionError> {
        self.snapshot(thread_id).await
    }

    async fn route_workspace(&self, working_dir: PathBuf) -> Result<String, LegacySessionError> {
        if let Some(session_id) = self
            .workspace_sessions
            .lock()
            .await
            .get(&working_dir)
            .cloned()
        {
            self.switch(session_id.clone()).await?;
            return Ok(session_id);
        }
        let current = self.create_with_messages(&[]).await?;
        self.workspace_sessions
            .lock()
            .await
            .insert(working_dir, current.session_id.clone());
        Ok(current.session_id)
    }
}

fn operation_error(error: impl std::fmt::Display) -> LegacySessionError {
    LegacySessionError::Operation(error.to_string())
}

async fn persist_legacy_message(manager: &mut SessionManager, message: &Message) {
    if message.content.len() != 1 {
        manager.push_message(message.clone()).await;
        return;
    }
    match (&message.role, &message.content[0]) {
        (Role::User, ContentBlock::Text { text }) => manager.push_user(text).await,
        (Role::Assistant, ContentBlock::Text { text }) => manager.push_assistant(text).await,
        (Role::System, ContentBlock::Text { text }) => manager.push_system(text),
        _ => manager.push_message(message.clone()).await,
    }
}
