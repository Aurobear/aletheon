//! Binary-owned compatibility adapter for the historical session RPC surface.
//!
//! This is a composition adapter, not a second Session authority: creation and
//! mutation dispatch through Runtime while `sessions.db` remains read-only.

use ::contracts::{Clock, ContentBlock, LlmProvider, Message, Role, SessionId};
use async_trait::async_trait;
use runtime::{ContextCompactorFactory, ContextWorkingSet};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

use runtime::session_service::SessionService;

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
    pub compaction: Option<runtime::ContextCompactionReceipt>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LegacyCompactionStatus {
    pub attempts: usize,
    pub successful: usize,
    pub last: Option<runtime::ContextCompactionReceipt>,
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
    async fn load_previous(
        &self,
        current_session_id: &str,
    ) -> Result<LegacySessionSnapshot, LegacySessionError>;
    async fn clear(&self, thread_id: &str) -> Result<LegacySessionTransition, LegacySessionError>;
    async fn compact(
        &self,
        thread_id: &str,
    ) -> Result<Option<LegacySessionTransition>, LegacySessionError>;
    async fn current(&self, thread_id: &str) -> Result<LegacySessionSnapshot, LegacySessionError>;
    async fn compaction_status(
        &self,
        thread_id: &str,
    ) -> Result<LegacyCompactionStatus, LegacySessionError>;
    async fn route_workspace(&self, working_dir: PathBuf) -> Result<String, LegacySessionError>;
}

pub struct LegacySessionService {
    registry: Arc<Mutex<HashMap<String, Arc<Mutex<ContextWorkingSet>>>>>,
    created_at: Arc<Mutex<HashMap<String, ::contracts::MonoTime>>>,
    workspace_sessions: Mutex<HashMap<PathBuf, String>>,
    data_dir: PathBuf,
    context_window: usize,
    clock: Arc<dyn Clock>,
    llm: Arc<dyn LlmProvider>,
    canonical: Arc<SessionService>,
    session_commands: Arc<dyn runtime::RuntimeCommandPort>,
}

pub struct LegacySessionResources {
    pub registry: Arc<Mutex<HashMap<String, Arc<Mutex<ContextWorkingSet>>>>>,
    pub created_at: Arc<Mutex<HashMap<String, ::contracts::MonoTime>>>,
    pub data_dir: PathBuf,
    pub context_window: usize,
    pub clock: Arc<dyn Clock>,
    pub llm: Arc<dyn LlmProvider>,
    pub canonical: Arc<SessionService>,
    pub session_commands: Arc<dyn runtime::RuntimeCommandPort>,
    /// Accepted only to keep old configuration/test construction source
    /// compatible. Runtime is always the sole live Session authority.
    pub session_writer: crate::config::SessionWriterMode,
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
            session_commands: resources.session_commands,
        }
    }

    async fn manager(
        &self,
        session_id: &str,
    ) -> Result<Arc<Mutex<ContextWorkingSet>>, LegacySessionError> {
        if let Some(manager) = self.registry.lock().await.get(session_id).cloned() {
            return Ok(manager);
        }
        let factory = mnemosyne::context_compactor::MnemosyneContextCompactorFactory;
        let manager = ContextWorkingSet::new(
            &self.data_dir,
            session_id.to_owned(),
            self.clock.clone(),
            factory.create(self.context_window, 80),
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
                ::contracts::wall_to_datetime(self.clock.wall_now())
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
        // Session creation always goes through the injected canonical Runtime
        // writer. Historical `sessions.db` is imported during bootstrap only.
        let session_id = self
            .session_commands
            .dispatch(runtime::RuntimeCommand::CreateSession(
                runtime::CreateSessionCommand {
                    correlation: None,
                    principal_hint: None,
                },
            ))
            .await
            .map_err(operation_error)?
            .session
            .ok_or_else(|| {
                LegacySessionError::Operation(
                    "Runtime create-session receipt omitted session id".into(),
                )
            })?
            .0;
        let factory = mnemosyne::context_compactor::MnemosyneContextCompactorFactory;
        let mut manager = ContextWorkingSet::new(
            &self.data_dir,
            session_id.clone(),
            self.clock.clone(),
            factory.create(self.context_window, 80),
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
            created_at: ::contracts::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
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
        Ok(LegacySessionTransition {
            previous,
            current,
            compaction: None,
        })
    }

    async fn list(&self) -> Result<Vec<LegacySessionView>, LegacySessionError> {
        // Query only the canonical projection. The process-local
        // ContextWorkingSet registry is a cache and historical rows have
        // already been imported during bootstrap.
        let now_ms = ::contracts::wall_to_datetime(self.clock.wall_now())
            .timestamp_millis()
            .max(0) as u64;
        let records = self
            .canonical
            .protocol_session_list()
            .await
            .map_err(operation_error)?
            .sessions;
        let mut result = Vec::with_capacity(records.len());
        for record in records.into_iter().take(100) {
            let message_count = self
                .canonical
                .items(&record.id)
                .await
                .map_err(operation_error)?
                .len();
            result.push(LegacySessionView {
                session_id: record.id.0,
                message_count,
                created_at: format!("{}ms ago", now_ms.saturating_sub(record.created_at_ms)),
            });
        }
        result.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        result.truncate(100);
        Ok(result)
    }

    async fn list_available(&self) -> Result<Vec<String>, LegacySessionError> {
        let mut ids = self
            .canonical
            .protocol_session_list()
            .await
            .map(|snapshot| {
                snapshot
                    .sessions
                    .into_iter()
                    .take(100)
                    .map(|session| session.id.0)
                    .collect::<Vec<_>>()
            })
            .map_err(operation_error)?;
        ids.sort();
        Ok(ids)
    }

    async fn switch(&self, session_id: String) -> Result<String, LegacySessionError> {
        let available = self
            .canonical
            .try_resume(&SessionId(session_id.clone()))
            .await
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
        if canonical.is_none() {
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
        let recent = self.list_available().await?.into_iter().next();
        match recent {
            Some(session_id) => self.resume(session_id).await,
            None => {
                let current = self.create_with_messages(&[]).await?;
                self.snapshot(&current.session_id).await
            }
        }
    }

    async fn load_previous(
        &self,
        current_session_id: &str,
    ) -> Result<LegacySessionSnapshot, LegacySessionError> {
        let previous = self
            .list_available()
            .await?
            .into_iter()
            .find(|session_id| session_id != current_session_id);
        match previous {
            Some(session_id) => self.resume(session_id).await,
            None => Err(LegacySessionError::NotFound(
                "no previous session is available".into(),
            )),
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
        Ok(LegacySessionTransition {
            previous,
            current,
            compaction: None,
        })
    }

    async fn compact(
        &self,
        thread_id: &str,
    ) -> Result<Option<LegacySessionTransition>, LegacySessionError> {
        let previous = self.current(thread_id).await?;
        let manager = self.manager(&previous.session_id).await?;
        let (messages, lineage, compaction) = {
            let mut manager = manager.lock().await;
            if !manager
                .force_compact(&*self.llm)
                .await
                .map_err(operation_error)?
            {
                return Ok(None);
            }
            (
                manager.history().to_vec(),
                manager.compaction_lineage().to_vec(),
                manager.compaction_lineage().last().cloned(),
            )
        };
        // Canonical history is immutable. Materialize the compacted view as a
        // new session rather than silently rewriting durable Session/Turn/Item truth.
        let current = self.create_with_messages(&messages).await?;
        self.manager(&current.session_id)
            .await?
            .lock()
            .await
            .inherit_compaction_lineage(&lineage);
        self.remap_default_workspace(&previous.session_id, &current.session_id)
            .await;
        Ok(Some(LegacySessionTransition {
            previous,
            current,
            compaction,
        }))
    }

    async fn current(&self, thread_id: &str) -> Result<LegacySessionSnapshot, LegacySessionError> {
        self.snapshot(thread_id).await
    }

    async fn compaction_status(
        &self,
        thread_id: &str,
    ) -> Result<LegacyCompactionStatus, LegacySessionError> {
        let current = self.current(thread_id).await?;
        let manager = self.manager(&current.session_id).await?;
        let manager = manager.lock().await;
        let lineage = manager.compaction_lineage();
        Ok(LegacyCompactionStatus {
            attempts: lineage.len(),
            successful: lineage.iter().filter(|entry| entry.applied).count(),
            last: lineage.last().cloned(),
        })
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

async fn persist_legacy_message(manager: &mut ContextWorkingSet, message: &Message) {
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
