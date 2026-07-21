//! Health and status RPC handlers.
//!
//! Methods: status, health.

use super::RequestHandler;
use serde_json::json;

impl RequestHandler {
    pub(super) async fn handle_conscious_diagnostics(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let Some(session_id) = request["params"]
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602,"message":"session_id is required"}});
        };
        let authority = crate::service::thread_authority::ThreadAuthorityKey::new(
            connection.principal_id.clone(),
            fabric::ThreadId(session_id.to_owned()),
        );
        match self.thread_authority.get(&authority) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32047,"message":"session is not visible to authenticated principal"}})
            }
            Err(error) => {
                return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32603,"message":error.to_string()}})
            }
        }
        let limit = request["params"]
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(50) as usize;
        match self
            .ports
            .conscious_workspaces
            .field_diagnostics(&fabric::AgoraSpaceId(session_id.to_owned()), limit)
        {
            Ok(Some(diagnostics)) => json!({"jsonrpc":"2.0", "id":id, "result":diagnostics}),
            Ok(None) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32048,"message":"conscious field has not started for session"}})
            }
            Err(error) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32603,"message":error.to_string()}})
            }
        }
    }

    pub(super) async fn handle_status(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let session_id = match request["params"].get("session_id").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32602, "message": "Missing session_id parameter" }
                })
            }
        };
        let turn_count = match self.ports.sessions.current(session_id).await {
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
        let mcp = self.mcp.as_ref().map(|manager| manager.health_snapshot());
        let external_status = mcp_external_status(mcp.as_ref());
        json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "status": health.production.readiness,
                "liveness": health.production.liveness,
                "readiness": health.production.readiness,
                "components": health.production.components,
                "external_dependencies": {
                    "status": external_status,
                    "mcp": mcp,
                },
                "uptime_seconds": health.uptime_seconds,
                "active_connections": health.active_connections,
                "session_count": session_count,
                "daemon_version": env!("CARGO_PKG_VERSION")
            }
        })
    }
}

fn mcp_external_status(
    snapshot: Option<&corpus::tools::mcp::supervisor::McpHealthSnapshot>,
) -> &'static str {
    if snapshot.is_some_and(|snapshot| {
        snapshot.servers.iter().any(|server| {
            matches!(
                server.state,
                corpus::tools::mcp::supervisor::McpServerHealthState::Connecting
                    | corpus::tools::mcp::supervisor::McpServerHealthState::Degraded
                    | corpus::tools::mcp::supervisor::McpServerHealthState::Reconnecting
            )
        })
    }) {
        "degraded"
    } else {
        "ready"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corpus::tools::mcp::supervisor::{
        McpHealthSnapshot, McpServerHealth, McpServerHealthState,
    };

    #[test]
    fn external_mcp_degradation_is_distinct_from_core_readiness() {
        assert_eq!(mcp_external_status(None), "ready");
        let snapshot = McpHealthSnapshot {
            accepting_tasks: true,
            servers: vec![McpServerHealth {
                server: "gbrain".into(),
                state: McpServerHealthState::Reconnecting,
                reason: Some("ping_failed".into()),
                reconnect_count: 1,
            }],
            tasks: Vec::new(),
        };
        assert_eq!(mcp_external_status(Some(&snapshot)), "degraded");
    }
}
