//! Session lifecycle RPC handlers.
//!
//! Methods: clear, sessions, resume, compact, new_session, load_recent,
//! session.create, session.list, session.switch.

use super::RequestHandler;

use serde_json::json;
use tracing::info;

use fabric::hook::{HookContext, HookPoint};
use std::collections::HashMap;

use crate::r#impl::daemon::session_manager::SessionManager;
use crate::session::store::SessionStore;

impl RequestHandler {
    pub(super) async fn handle_clear(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let (session_id, sm_arc) = match self.get_or_create_session(None).await {
            Ok(v) => v,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": e.to_string() }
                })
            }
        };
        // Fire OnSessionEnd hook before clearing
        {
            let (sid, turn_count) = {
                let sm = sm_arc.lock().await;
                (sm.session_id.clone(), sm.turn_count())
            };
            let hr = self.subsystems.corpus.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnSessionEnd,
                session_id: sid,
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }
        // Run configured on_session_end hook scripts
        if !self
            .subsystems
            .corpus
            .hooks_config
            .on_session_end
            .is_empty()
        {
            let hook_input = serde_json::json!({
                "session_id": &session_id,
                "cwd": std::env::current_dir().unwrap_or_default()
            });
            let _ = self
                .run_hook_scripts(
                    &self.subsystems.corpus.hooks_config.on_session_end,
                    &hook_input.to_string(),
                )
                .await;
        }
        // Distill session facts into FactStore
        {
            let fs = self.subsystems.memory.fact_store.lock().await;
            let sm = sm_arc.lock().await;
            let recent: Vec<_> = sm.history().iter().rev().take(10).collect();
            for msg in &recent {
                if matches!(msg.role, fabric::Role::User) {
                    for block in &msg.content {
                        if let fabric::ContentBlock::Text { text } = block {
                            if text.len() > 20 {
                                let lower = text.to_lowercase();
                                if lower.contains("prefer")
                                    || lower.contains("always")
                                    || lower.contains("never")
                                    || lower.contains("remember")
                                {
                                    let _ =
                                        fs.add_fact(text, "session", "", "", 0.6, "episodic", 14);
                                }
                            }
                        }
                    }
                }
            }
            let _ = fs.decay_stale();
        }
        // Clear both the live history and its durable recovery tail. Keeping
        // only the existing hook/distillation behavior here leaked previous
        // turns into the next TUI invocation for the same workspace.
        {
            let mut sm = sm_arc.lock().await;
            if let Err(e) = sm.clear_history().await {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32022, "message": format!("Session clear error: {e}") }
                });
            }
        }
        // Clear cancel token
        {
            let mut ct = self.subsystems.cancel_token.lock().await;
            *ct = None;
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "status": "ok" }
        })
    }

    pub(super) async fn handle_sessions_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match SessionStore::new(&self.subsystems.session.data_dir) {
            Ok(store) => match store.list_sessions() {
                Ok(ids) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "sessions": ids }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": format!("Session list error: {}", e) }
                }),
            },
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": format!("SessionStore init error: {}", e) }
            }),
        }
    }

    pub(super) async fn handle_resume(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let target_session_id = request["params"]["session_id"].as_str().unwrap_or("");
        if target_session_id.is_empty() {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32021, "message": "Missing session_id parameter" }
            })
        } else {
            match SessionManager::recover(&self.subsystems.session.data_dir, target_session_id)
                .await
            {
                Some(msgs) => {
                    match SessionManager::new(
                        &self.subsystems.session.data_dir,
                        target_session_id.to_string(),
                        self.subsystems.session.context_window,
                        self.subsystems.kernel.clock(),
                    )
                    .await
                    {
                        Ok(new_sm) => {
                            let msg_count = msgs.len();
                            let sid = target_session_id.to_string();
                            self.register_default_session(sid, new_sm).await;
                            info!(
                                session_id = target_session_id,
                                messages = msg_count,
                                "Session resumed"
                            );
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "session_id": target_session_id,
                                    "recovered_messages": msg_count,
                                }
                            })
                        }
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32021, "message": format!("SessionManager init error: {}", e) }
                        }),
                    }
                }
                None => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32021, "message": format!("No recoverable session: {}", target_session_id) }
                }),
            }
        }
    }

    pub(super) async fn handle_compact(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let did_compact = {
            let (_sid, sm_arc) = match self.get_or_create_session(None).await {
                Ok(v) => v,
                Err(e) => {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": e.to_string() }
                    })
                }
            };
            let mut sm = sm_arc.lock().await;
            // Force compaction by temporarily lowering threshold
            sm.force_compact(&*self.llm).await
        };
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "compacted": did_compact }
        })
    }

    pub(super) async fn handle_new_session(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let new_id = uuid::Uuid::new_v4().to_string();
        // Get current session info for hooks
        let (old_id, turn_count, old_hook_session_id) = {
            let (_sid, sm_arc) = match self.get_or_create_session(None).await {
                Ok(v) => v,
                Err(e) => {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": e.to_string() }
                    })
                }
            };
            let sm = sm_arc.lock().await;
            (
                sm.session_id.clone(),
                sm.turn_count(),
                sm.session_id.clone(),
            )
        };
        // Fire OnSessionEnd for the outgoing session
        {
            let hr = self.subsystems.corpus.hook_registry.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnSessionEnd,
                session_id: old_id,
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }
        // Run configured on_session_end hook scripts
        if !self
            .subsystems
            .corpus
            .hooks_config
            .on_session_end
            .is_empty()
        {
            let hook_input = serde_json::json!({
                "session_id": &old_hook_session_id,
                "cwd": std::env::current_dir().unwrap_or_default()
            });
            let _ = self
                .run_hook_scripts(
                    &self.subsystems.corpus.hooks_config.on_session_end,
                    &hook_input.to_string(),
                )
                .await;
        }
        // Create new session and replace SessionManager
        match SessionManager::new(
            &self.subsystems.session.data_dir,
            new_id.clone(),
            self.subsystems.session.context_window,
            self.subsystems.kernel.clock(),
        )
        .await
        {
            Ok(new_sm) => {
                // Register session in store
                if let Ok(store) = SessionStore::new(&self.subsystems.session.data_dir) {
                    let _ = store.create_session(&new_id);
                }
                self.register_default_session(new_id.clone(), new_sm).await;
                // Clear per-session approval cache
                self.subsystems
                    .security
                    .session_approvals
                    .lock()
                    .await
                    .clear();
                // Fire OnSessionStart for the new session
                {
                    let hr = self.subsystems.corpus.hook_registry.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::OnSessionStart,
                        session_id: new_id.clone(),
                        turn_count: 0,
                        tool_name: None,
                        tool_input: None,
                        tool_result: None,
                        message: None,
                        metadata: HashMap::new(),
                    };
                    hr.execute(&ctx).await;
                }
                info!(session_id = %new_id, "New session created");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "session_id": new_id }
                })
            }
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32030, "message": format!("Failed to create session: {}", e) }
            }),
        }
    }

    pub(super) async fn handle_load_recent(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match SessionStore::new(&self.subsystems.session.data_dir) {
            Ok(store) => match store.most_recent() {
                Ok(Some(recent_id)) => {
                    match SessionManager::recover(&self.subsystems.session.data_dir, &recent_id)
                        .await
                    {
                        Some(msgs) => {
                            match SessionManager::new(
                                &self.subsystems.session.data_dir,
                                recent_id.clone(),
                                self.subsystems.session.context_window,
                                self.subsystems.kernel.clock(),
                            )
                            .await
                            {
                                Ok(new_sm) => {
                                    let msg_count = msgs.len();
                                    self.register_default_session(recent_id.clone(), new_sm)
                                        .await;
                                    info!(
                                        session_id = %recent_id,
                                        messages = msg_count,
                                        "Loaded most recent session"
                                    );
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "session_id": recent_id,
                                            "recovered_messages": msg_count,
                                        }
                                    })
                                }
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                                }),
                            }
                        }
                        None => {
                            // No recoverable journal -- create fresh session with this id
                            match SessionManager::new(
                                &self.subsystems.session.data_dir,
                                recent_id.clone(),
                                self.subsystems.session.context_window,
                                self.subsystems.kernel.clock(),
                            )
                            .await
                            {
                                Ok(new_sm) => {
                                    self.register_default_session(recent_id.clone(), new_sm)
                                        .await;
                                    info!(session_id = %recent_id, "Loaded recent session (no journal, fresh)");
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "session_id": recent_id,
                                            "recovered_messages": 0,
                                        }
                                    })
                                }
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                                }),
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No sessions exist at all -- create a new one
                    let new_id = uuid::Uuid::new_v4().to_string();
                    match SessionManager::new(
                        &self.subsystems.session.data_dir,
                        new_id.clone(),
                        self.subsystems.session.context_window,
                        self.subsystems.kernel.clock(),
                    )
                    .await
                    {
                        Ok(new_sm) => {
                            if let Ok(store) = SessionStore::new(&self.subsystems.session.data_dir)
                            {
                                let _ = store.create_session(&new_id);
                            }
                            self.register_default_session(new_id.clone(), new_sm).await;
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": { "session_id": new_id, "recovered_messages": 0 }
                            })
                        }
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                        }),
                    }
                }
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32031, "message": format!("SessionStore query error: {}", e) }
                }),
            },
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32031, "message": format!("SessionStore init error: {}", e) }
            }),
        }
    }

    pub(super) async fn handle_session_create(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.sessions.create().await {
            Ok(session) => {
                json!({"jsonrpc":"2.0", "id":id, "result":{"session_id":session.session_id,"created_at":session.created_at}})
            }
            Err(error) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32030,"message":format!("Failed to create session: {error}")}})
            }
        }
    }

    pub(super) async fn handle_session_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.sessions.list().await {
            Ok(sessions) => json!({"jsonrpc":"2.0", "id":id, "result":sessions}),
            Err(error) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32020,"message":error.to_string()}})
            }
        }
    }

    pub(super) async fn handle_session_switch(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let target = request["params"]["session_id"].as_str().unwrap_or("");
        if target.is_empty() {
            return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602,"message":"Missing session_id parameter"}});
        }
        match self.ports.sessions.switch(target.to_string()).await {
            Ok(session_id) => {
                json!({"jsonrpc":"2.0", "id":id, "result":{"session_id":session_id,"status":"switched"}})
            }
            Err(crate::service::legacy_session_service::LegacySessionError::NotFound(_)) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32021,"message":format!("Session not found: {target}")}})
            }
            Err(error) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32020,"message":error.to_string()}})
            }
        }
    }
}
