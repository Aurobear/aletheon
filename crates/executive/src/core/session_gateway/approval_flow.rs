//! Operational handlers — parameter management and audit journal access.
//!
//! Contains handlers for dynamic parameter get/list (Phase A) and the session
//! journal query (Phase E).

use serde_json::{json, Value};
use tracing::warn;

use super::gateway::SessionGateway;

impl SessionGateway {
    // ── Phase A: Param methods ───────────────────────────────────────────

    pub(crate) async fn handle_param_get(&self, id: &Value, params: &Value) -> Value {
        let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");

        match self.param_registry.get(key).await {
            Some(value) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "key": key, "value": value }
            }),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32050, "message": format!("Unknown parameter: {}", key) }
            }),
        }
    }

    pub(crate) async fn handle_param_list(&self, id: &Value, params: &Value) -> Value {
        let namespace = params.get("namespace").and_then(|v| v.as_str());
        let values = self.param_registry.list(namespace).await;

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "params": values }
        })
    }

    // ── Phase E: Journal ────────────────────────────────────────────────────

    pub(crate) async fn handle_journal(&self, id: &Value, params: &Value) -> Value {
        let event_type = params.get("event_type").and_then(|v| v.as_str());
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let session_id = fabric::SessionId(self.session_manager.lock().await.session_id.clone());
        match self.canonical_sessions.items(&session_id).await {
            Ok(entries) => {
                let mut rendered: Vec<Value> = entries
                    .into_iter()
                    .filter_map(|entry| {
                        let event_type_str = match &entry.payload {
                            fabric::ItemPayload::UserMessage { .. } => "user_message",
                            fabric::ItemPayload::AssistantMessage { .. } => "assistant_message",
                            fabric::ItemPayload::ToolCall { .. } => "tool_call",
                            fabric::ItemPayload::ToolResult { .. } => "tool_result",
                            fabric::ItemPayload::ContextProjection { .. } => "context_projection",
                            fabric::ItemPayload::SystemNotice { .. } => "system_notice",
                        };
                        if event_type.is_some_and(|expected| expected != event_type_str) {
                            return None;
                        }
                        json!({
                            "sequence": entry.sequence,
                            "timestamp": fabric::wall_to_datetime(fabric::WallTime(entry.created_at_ms as i64)).to_rfc3339(),
                            "event_type": event_type_str,
                            "item": entry,
                        })
                        .into()
                    })
                    .collect();
                if let Some(limit) = limit {
                    rendered.truncate(limit);
                }

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "count": rendered.len(),
                        "entries": rendered,
                    }
                })
            }
            Err(e) => {
                warn!(error = %e, "session.journal: query failed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32054, "message": format!("Journal query failed: {}", e) }
                })
            }
        }
    }
}
