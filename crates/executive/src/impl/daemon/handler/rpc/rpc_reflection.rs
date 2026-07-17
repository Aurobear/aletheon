//! Reflection and self-awareness RPC handlers.
//!
//! Methods: reflect, reflect_now, genome, evolution.

use super::RequestHandler;
use serde_json::json;
use tracing::{info, warn};

impl RequestHandler {
    pub(super) async fn handle_reflect(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.reflection.list(10).await {
            Ok(entries) => json!({"jsonrpc":"2.0","id":id,"result":{"reflections":entries}}),
            Err(error) => {
                warn!(%error, "Failed to recall reflections");
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":format!("Reflection recall error: {error}")}})
            }
        }
    }

    pub(super) async fn handle_reflect_now(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let session_id = match request["params"].get("session_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Missing session_id parameter"}}),
        };
        let turn = match self.ports.sessions.current(session_id).await {
            Ok(snapshot) => snapshot.turn_count,
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
            }
        };
        match self.ports.reflection.reflect_now(turn).await {
            Ok(entry) => {
                info!(reflection_id = %entry.id, "Manual reflection stored via reflect_now");
                json!({"jsonrpc":"2.0","id":id,"result":{"reflection":{
                    "id":entry.id,"timestamp":entry.timestamp.to_rfc3339(),
                    "task_summary":entry.task_summary,"outcome":entry.outcome.to_string(),
                    "what_worked":entry.what_worked,"what_failed":entry.what_failed,
                    "learned":entry.learned,"confidence":entry.confidence,"turn_count":turn
                }}})
            }
            Err(error) => {
                warn!(%error, "Failed to store manual reflection");
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32003,"message":format!("Reflect now error: {error}")}})
            }
        }
    }

    pub(super) async fn handle_genome(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.reflection.genome_yaml().await {
            Ok(genome) => json!({"jsonrpc":"2.0","id":id,"result":{"genome":genome}}),
            Err(error) => {
                warn!(%error, "Failed to read genome");
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32004,"message":format!("Genome read error: {error}")}})
            }
        }
    }

    pub(super) async fn handle_evolution(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.reflection.evolution(20).await {
            Ok(entries) => {
                json!({"jsonrpc":"2.0","id":id,"result":{"evolution":entries,"current_version":"0.1.0"}})
            }
            Err(error) => {
                warn!(%error, "Failed to recall evolution logs");
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":format!("Evolution recall error: {error}")}})
            }
        }
    }
}
