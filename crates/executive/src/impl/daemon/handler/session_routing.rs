//! Session routing and lifecycle management.
//!
//! Manages the multi-session registry: session creation, lookup,
//! default session switching, and recovery from persistent storage.

use super::super::session_manager::SessionManager;
use super::RequestHandler;
use anyhow::anyhow;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

impl RequestHandler {
    /// Get or create a session by ID. Returns the session_id and the session.
    /// If `session_id` is `None`, uses the default session for this connection.
    pub(crate) async fn get_or_create_session(
        &self,
        session_id: Option<&str>,
    ) -> anyhow::Result<(String, Arc<Mutex<SessionManager>>)> {
        let id = if let Some(sid) = session_id {
            sid.to_string()
        } else {
            self.subsystems
                .session
                .default_session_id
                .lock()
                .await
                .clone()
        };

        // Fast path: check if session exists
        {
            let sessions = self.sessions.lock().await;
            if let Some(sm) = sessions.get(&id) {
                return Ok((id, sm.clone()));
            }
        }

        // Slow path: create session on demand
        match SessionManager::new(
            &self.subsystems.session.data_dir,
            id.clone(),
            self.subsystems.session.context_window,
            self.subsystems.kernel.clock(),
        )
        .await
        {
            Ok(new_sm) => {
                let sm = Arc::new(Mutex::new(new_sm));
                let mut sessions = self.sessions.lock().await;
                sessions.insert(id.clone(), sm.clone());
                self.subsystems
                    .session
                    .session_created_at
                    .lock()
                    .await
                    .insert(id.clone(), self.subsystems.kernel.clock().mono_now());
                info!(session_id = %id, "Session created on demand");
                Ok((id, sm))
            }
            Err(e) => {
                warn!(error = %e, session_id = %id, "Failed to create session on demand, falling back to default");
                let default_id = self
                    .subsystems
                    .session
                    .default_session_id
                    .lock()
                    .await
                    .clone();
                let sessions = self.sessions.lock().await;
                let sm = sessions
                    .get(&default_id)
                    .ok_or_else(|| {
                        anyhow!("default session '{}' not found in registry", default_id)
                    })?
                    .clone();
                Ok((default_id, sm))
            }
        }
    }

    /// Register a session in the registry and set it as the default.
    pub(crate) async fn register_default_session(
        &self,
        session_id: String,
        session_manager: SessionManager,
    ) {
        let sm = Arc::new(Mutex::new(session_manager));
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id.clone(), sm);
        *self.subsystems.session.default_session_id.lock().await = session_id.clone();
        self.subsystems
            .session
            .session_created_at
            .lock()
            .await
            .insert(
                session_id.clone(),
                self.subsystems.kernel.clock().mono_now(),
            );
    }
}
