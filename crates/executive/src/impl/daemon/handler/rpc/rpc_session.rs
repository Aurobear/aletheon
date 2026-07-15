//! Session lifecycle RPC handlers.
//!
//! All session mechanics are delegated to `LegacySessionUseCases`; this file
//! only adapts the compatibility JSON-RPC shapes and lifecycle side effects.

use super::RequestHandler;

use serde_json::json;
use tracing::info;

use crate::service::legacy_session_service::{LegacySessionError, LegacySessionSnapshot};

impl RequestHandler {
    pub(super) async fn handle_clear(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let transition = match self.ports.sessions.clear().await {
            Ok(transition) => transition,
            Err(error) => return session_error(id, -32022, error),
        };
        self.finish_outgoing_session(&transition.previous).await;
        self.distill_session_facts(&transition.previous).await;
        self.ports.session_lifecycle.reset_turn_token().await;
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "status": "ok",
                "session_id": transition.current.session_id,
            }
        })
    }

    pub(super) async fn handle_sessions_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.sessions.list_available().await {
            Ok(sessions) => json!({"jsonrpc":"2.0", "id":id, "result":{"sessions":sessions}}),
            Err(error) => session_error(id, -32020, error),
        }
    }

    pub(super) async fn handle_resume(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let target = request["params"]["session_id"].as_str().unwrap_or("");
        if target.is_empty() {
            return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32021,"message":"Missing session_id parameter"}});
        }
        match self.ports.sessions.resume(target.to_owned()).await {
            Ok(snapshot) => {
                info!(session_id = %snapshot.session_id, messages = snapshot.messages.len(), "Session resumed");
                json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "result":{
                        "session_id":snapshot.session_id,
                        "recovered_messages":snapshot.messages.len(),
                    }
                })
            }
            Err(error) => session_error(id, -32021, error),
        }
    }

    pub(super) async fn handle_compact(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.sessions.compact().await {
            Ok(Some(transition)) => json!({
                "jsonrpc":"2.0",
                "id":id,
                "result":{
                    "compacted":true,
                    "session_id":transition.current.session_id,
                }
            }),
            Ok(None) => json!({"jsonrpc":"2.0", "id":id, "result":{"compacted":false}}),
            Err(error) => session_error(id, -32023, error),
        }
    }

    pub(super) async fn handle_new_session(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let transition = match self.ports.sessions.create_and_switch().await {
            Ok(transition) => transition,
            Err(error) => return session_error(id, -32030, error),
        };
        self.finish_outgoing_session(&transition.previous).await;
        self.ports
            .session_lifecycle
            .start(transition.current.session_id.clone(), true)
            .await;
        info!(session_id = %transition.current.session_id, "New session created");
        json!({"jsonrpc":"2.0", "id":id, "result":{"session_id":transition.current.session_id}})
    }

    pub(super) async fn handle_load_recent(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.sessions.load_recent().await {
            Ok(snapshot) => {
                info!(session_id = %snapshot.session_id, messages = snapshot.messages.len(), "Loaded most recent session");
                json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "result":{
                        "session_id":snapshot.session_id,
                        "recovered_messages":snapshot.messages.len(),
                    }
                })
            }
            Err(error) => session_error(id, -32031, error),
        }
    }

    async fn finish_outgoing_session(&self, snapshot: &LegacySessionSnapshot) {
        self.ports
            .session_lifecycle
            .finish(snapshot.session_id.clone(), snapshot.turn_count)
            .await;
    }

    async fn distill_session_facts(&self, snapshot: &LegacySessionSnapshot) {
        for message in snapshot.messages.iter().rev().take(10) {
            if !matches!(message.role, fabric::Role::User) {
                continue;
            }
            for block in &message.content {
                let fabric::ContentBlock::Text { text } = block else {
                    continue;
                };
                let lower = text.to_lowercase();
                if text.len() > 20
                    && ["prefer", "always", "never", "remember"]
                        .iter()
                        .any(|keyword| lower.contains(keyword))
                {
                    let _ = self
                        .ports
                        .facts
                        .add(mnemosyne::AddFactRequest {
                            content: text.clone(),
                            scope: "session".into(),
                            subject: snapshot.session_id.clone(),
                            tags: "distilled".into(),
                        })
                        .await;
                }
            }
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

fn session_error(
    id: &serde_json::Value,
    code: i64,
    error: LegacySessionError,
) -> serde_json::Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":error.to_string()}})
}
