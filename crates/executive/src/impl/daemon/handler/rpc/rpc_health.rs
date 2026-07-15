//! Health and status RPC handlers.
//!
//! Methods: status, health.

use super::RequestHandler;
use serde_json::json;

impl RequestHandler {
    pub(super) async fn handle_status(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let turn_count = match self.ports.sessions.current().await {
            Ok(snapshot) => snapshot.turn_count,
            Err(error) => {
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32000, "message": error.to_string() }
                });
            }
        };
        match self.ports.health.status().await {
            Ok(status) => json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "status": {
                    "session_id": status.session_id,
                    "turn_count": turn_count,
                    "iteration": status.iteration,
                    "reflection_count": status.reflection_count,
                    "evolution_count": status.evolution_count,
                    "care_weights": status.care_weights,
                    "boundary_rules": status.boundary_rules,
                    "boundary_immutable": status.boundary_immutable,
                    "attention_focus": status.attention_focus,
                }}
            }),
            Err(error) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32000, "message": error.to_string() }
            }),
        }
    }

    pub(super) async fn handle_health(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let session_count = self
            .ports
            .sessions
            .list()
            .await
            .map_or(0, |items| items.len());
        let health = self.ports.health.health().await;
        json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "status": health.production.readiness,
                "liveness": health.production.liveness,
                "readiness": health.production.readiness,
                "components": health.production.components,
                "uptime_seconds": health.uptime_seconds,
                "active_connections": health.active_connections,
                "session_count": session_count,
                "daemon_version": env!("CARGO_PKG_VERSION")
            }
        })
    }
}
