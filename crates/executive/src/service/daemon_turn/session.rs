//! Session-management methods on `DaemonTurnOrchestrator` and `TurnPipeline`.

use super::orchestrator::DaemonTurnOrchestrator;
use crate::r#impl::daemon::session_manager::SessionManager;
use crate::service::turn_pipeline::TurnPipeline;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[allow(dead_code)]
impl DaemonTurnOrchestrator {
    pub(crate) async fn get_or_create_session(
        &self,
        session_id: Option<&str>,
    ) -> (String, Arc<Mutex<SessionManager>>) {
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
        {
            let sessions = self.sessions.lock().await;
            if let Some(sm) = sessions.get(&id) {
                return (id, sm.clone());
            }
        }
        match SessionManager::new(
            &self.subsystems.session.data_dir,
            id.clone(),
            self.subsystems.session.context_window,
            self.clock.clone(),
        )
        .await
        {
            Ok(sm) => {
                let sm_arc = Arc::new(Mutex::new(sm));
                self.sessions
                    .lock()
                    .await
                    .insert(id.clone(), sm_arc.clone());
                self.subsystems
                    .session
                    .session_created_at
                    .lock()
                    .await
                    .insert(id.clone(), self.clock.mono_now());
                info!(session_id = %id, "Session created on demand");
                (id, sm_arc)
            }
            Err(e) => {
                warn!(error = %e, session_id = %id, "Failed to create session on demand, using default");
                let default_id = self
                    .subsystems
                    .session
                    .default_session_id
                    .lock()
                    .await
                    .clone();
                let sessions = self.sessions.lock().await;
                let sm = sessions.get(&default_id).cloned().unwrap();
                (default_id, sm)
            }
        }
    }

    pub(crate) async fn begin_turn_token(&self) -> CancellationToken {
        let ct = CancellationToken::new();
        let mut token = self.subsystems.cancel_token.lock().await;
        *token = Some(ct.clone());
        ct
    }
}

impl TurnPipeline {
    pub(crate) async fn get_or_create_session(
        &self,
        session_id: Option<&str>,
    ) -> (String, Arc<Mutex<SessionManager>>) {
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
        {
            let sessions = self.sessions.lock().await;
            if let Some(sm) = sessions.get(&id) {
                return (id, sm.clone());
            }
        }
        match SessionManager::new(
            &self.subsystems.session.data_dir,
            id.clone(),
            self.subsystems.session.context_window,
            self.clock.clone(),
        )
        .await
        {
            Ok(sm) => {
                let sm_arc = Arc::new(Mutex::new(sm));
                self.sessions
                    .lock()
                    .await
                    .insert(id.clone(), sm_arc.clone());
                self.subsystems
                    .session
                    .session_created_at
                    .lock()
                    .await
                    .insert(id.clone(), self.clock.mono_now());
                info!(session_id = %id, "Session created on demand");
                (id, sm_arc)
            }
            Err(e) => {
                warn!(error = %e, session_id = %id, "Failed to create session on demand, using default");
                let default_id = self
                    .subsystems
                    .session
                    .default_session_id
                    .lock()
                    .await
                    .clone();
                let sessions = self.sessions.lock().await;
                let sm = sessions.get(&default_id).cloned().unwrap();
                (default_id, sm)
            }
        }
    }
}
