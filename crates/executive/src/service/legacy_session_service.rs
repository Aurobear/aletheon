//! Request-safe legacy session registry use cases.

use crate::r#impl::daemon::session_manager::SessionManager;
use async_trait::async_trait;
use fabric::Clock;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LegacySessionView {
    pub session_id: String,
    pub message_count: usize,
    pub created_at: String,
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
    async fn list(&self) -> Result<Vec<LegacySessionView>, LegacySessionError>;
    async fn switch(&self, session_id: String) -> Result<String, LegacySessionError>;
}

pub struct LegacySessionService {
    registry: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    default_id: Arc<Mutex<String>>,
    created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    data_dir: PathBuf,
    context_window: usize,
    clock: Arc<dyn Clock>,
}

impl LegacySessionService {
    pub fn new(
        registry: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
        default_id: Arc<Mutex<String>>,
        created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
        data_dir: PathBuf,
        context_window: usize,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            registry,
            default_id,
            created_at,
            data_dir,
            context_window,
            clock,
        }
    }
}

#[async_trait]
impl LegacySessionUseCases for LegacySessionService {
    async fn create(&self) -> Result<LegacySessionView, LegacySessionError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let manager = SessionManager::new(
            &self.data_dir,
            session_id.clone(),
            self.context_window,
            self.clock.clone(),
        )
        .await
        .map_err(|error| LegacySessionError::Operation(error.to_string()))?;
        self.registry
            .lock()
            .await
            .insert(session_id.clone(), Arc::new(Mutex::new(manager)));
        self.created_at
            .lock()
            .await
            .insert(session_id.clone(), self.clock.mono_now());
        Ok(LegacySessionView {
            session_id,
            message_count: 0,
            created_at: fabric::wall_to_datetime(self.clock.wall_now()).to_rfc3339(),
        })
    }

    async fn list(&self) -> Result<Vec<LegacySessionView>, LegacySessionError> {
        let registry = self.registry.lock().await;
        let created = self.created_at.lock().await;
        let now = self.clock.mono_now().0;
        let mut result = Vec::with_capacity(registry.len().min(100));
        for (session_id, manager) in registry.iter().take(100) {
            let message_count = manager.lock().await.message_count();
            let created_at = created
                .get(session_id)
                .map(|time| format!("{}s ago", now.saturating_sub(time.0) / 1000))
                .unwrap_or_else(|| "unknown".into());
            result.push(LegacySessionView {
                session_id: session_id.clone(),
                message_count,
                created_at,
            });
        }
        result.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        Ok(result)
    }

    async fn switch(&self, session_id: String) -> Result<String, LegacySessionError> {
        if !self.registry.lock().await.contains_key(&session_id) {
            return Err(LegacySessionError::NotFound(session_id));
        }
        *self.default_id.lock().await = session_id.clone();
        Ok(session_id)
    }
}
