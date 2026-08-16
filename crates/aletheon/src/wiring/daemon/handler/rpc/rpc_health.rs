//! Health and status RPC handlers.
//!
//! Methods: status, health.

use super::RequestHandler;
use serde_json::json;

impl RequestHandler {
    pub(crate) async fn status_projection(
        &self,
        session_id: &str,
    ) -> anyhow::Result<::contracts::contract::command::StatusProjectionV1> {
        let turn_count = self.ports.sessions.current(session_id).await?.turn_count;
        let compaction = self.ports.sessions.compaction_status(session_id).await?;
        let status = self.ports.health.status().await?;
        let memory = self.ports.memory_health_snapshot();

        Ok(::contracts::contract::command::StatusProjectionV1 {
            session_id: session_id.to_owned(),
            turn_count,
            iteration: status.iteration,
            reflection_count: status.reflection_count,
            evolution_count: status.evolution_count,
            care_weights: status
                .care_weights
                .into_iter()
                .map(|care| ::contracts::contract::command::StatusCareWeightV1 {
                    topic: care.topic,
                    weight: care.weight,
                })
                .collect(),
            boundary_rules: status.boundary_rules,
            boundary_immutable: status.boundary_immutable,
            attention_focus: status.attention_focus,
            compaction: ::contracts::contract::command::StatusCompactionV1 {
                attempts: compaction.attempts,
                successful: compaction.successful,
                last: compaction.last.map(|run| {
                    ::contracts::contract::command::StatusCompactionRunV1 {
                        run_id: run.run_id,
                        strategy: format!("{:?}", run.strategy).to_ascii_lowercase(),
                        tokens_before: run.tokens_before,
                        tokens_after: run.tokens_after,
                        forced: run.forced,
                        applied: run.applied,
                        failure: run.failure,
                    }
                }),
            },
            memory: ::contracts::contract::command::StatusMemoryV1 {
                provider: "composite".into(),
                local: "healthy".into(),
                supplemental: ::contracts::contract::command::StatusSupplementalMemoryV1 {
                    enabled: memory.supplemental_enabled,
                    state: if memory.degraded {
                        "degraded".into()
                    } else {
                        "healthy".into()
                    },
                    error_category: memory
                        .error_category
                        .map(|value| format!("{value:?}").to_ascii_lowercase()),
                    queue_depth: memory.queue_depth,
                },
            },
        })
    }

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
        let authority = application::thread_authority::ThreadAuthorityKey::new(
            connection.principal_id.clone(),
            ::contracts::ThreadId(session_id.to_owned()),
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
            .field_diagnostics(&::contracts::AgoraSpaceId(session_id.to_owned()), limit)
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
        let turn_watchdog = self.ports.turn.watchdog_snapshot().await;
        let local_providers =
            crate::wiring::adapters::inference::backpressure::all_provider_backpressure_snapshots();
        let (core_providers, provider_metrics_error) =
            match self.ports.inference.provider_backpressure_metrics().await {
                Ok(providers) => (providers, None),
                Err(error) => (Default::default(), Some(error.to_string())),
            };
        let mut alerts = Vec::new();
        if turn_watchdog.overdue > 0 || turn_watchdog.stale_without_deadline > 0 {
            alerts.push("turn_watchdog_stale");
        }
        if core_providers
            .values()
            .chain(local_providers.values())
            .any(|provider| provider.rejected > 0)
        {
            alerts.push("provider_backpressure_rejected");
        }
        if provider_metrics_error.is_some() {
            alerts.push("provider_metrics_unavailable");
        }
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
                "metrics": {
                    "provider_backpressure": {
                        "machine_core": core_providers,
                        "user_runtime": local_providers,
                        "error": provider_metrics_error,
                    },
                    "turn_watchdog": turn_watchdog,
                },
                "operator_slo": {
                    "stale_turn_after_ms": 900000,
                    "provider_rejection_alert_threshold": 1,
                    "readiness_required": "ready",
                    "alerts": alerts
                },
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
