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

        let sm = self.session_manager.lock().await;
        let journal = sm.journal();

        match journal.query(None, None, event_type, limit) {
            Ok(entries) => {
                let rendered: Vec<Value> = entries
                    .iter()
                    .map(|entry| {
                        let event_type_str = match &entry.event {
                            crate::r#impl::session::journal::SessionEvent::SessionCreated {
                                ..
                            } => "session_created",
                            crate::r#impl::session::journal::SessionEvent::UserMessage {
                                ..
                            } => "user_message",
                            crate::r#impl::session::journal::SessionEvent::AssistantMessage {
                                ..
                            } => "assistant_message",
                            crate::r#impl::session::journal::SessionEvent::ToolUseBlock {
                                ..
                            } => "tool_use_block",
                            crate::r#impl::session::journal::SessionEvent::ToolResultBlock {
                                ..
                            } => "tool_result_block",
                            crate::r#impl::session::journal::SessionEvent::ToolCallStarted {
                                ..
                            } => "tool_call_started",
                            crate::r#impl::session::journal::SessionEvent::ToolCallCompleted {
                                ..
                            } => "tool_call_completed",
                            crate::r#impl::session::journal::SessionEvent::CheckpointBoundary {
                                ..
                            } => "checkpoint_boundary",
                            crate::r#impl::session::journal::SessionEvent::Compacted { .. } => {
                                "compacted"
                            }
                            crate::r#impl::session::journal::SessionEvent::Summary { .. } => {
                                "summary"
                            }
                            crate::r#impl::session::journal::SessionEvent::SessionEnded {
                                ..
                            } => "session_ended",
                        };
                        json!({
                            "timestamp": entry.timestamp.to_rfc3339(),
                            "event_type": event_type_str,
                            "event": entry.event,
                        })
                    })
                    .collect();

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
