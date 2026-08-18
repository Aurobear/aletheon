//! Admin and meta RPC handlers.

use super::RequestHandler;
use crate::host::admin_service::{AdminServiceError, TransientApprovalRequest};
use application::turn_control::{CollaborationMode, InterruptReason};
use serde_json::json;
use tracing::info;

impl RequestHandler {
    pub(super) async fn handle_diff_artifact_get(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = match serde_json::from_value::<
            ::contracts::protocol::client::DiffArtifactGetParams,
        >(request.get("params").cloned().unwrap_or_default())
        {
            Ok(params) if params.limit > 0 && params.limit <= 64 * 1024 => params,
            Ok(_) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"diff page limit must be 1..=65536"}})
            }
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":error.to_string()}})
            }
        };
        let store = corpus::tools::artifact::ArtifactStore::new(
            corpus::tools::tools::output::OutputConfig::default()
                .overflow_dir
                .join("artifacts"),
        );
        match store.read_page(&params.sha256, params.offset, params.limit as u64) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(content) => json!({"jsonrpc":"2.0","id":id,"result":{
                    "sha256":params.sha256,"offset":params.offset,
                    "next_offset":params.offset.saturating_add(content.len() as u64),
                    "eof":content.len() < params.limit as usize,"content":content}}),
                Err(_) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32041,"message":"diff artifact is not UTF-8"}})
                }
            },
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32040,"message":error.to_string()}})
            }
        }
    }
    pub(super) async fn handle_deployment_rollback(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = &request["params"];
        if params.get("confirm").and_then(|value| value.as_str())
            != Some("restore-previous-deployment")
        {
            return json!({
                "jsonrpc":"2.0", "id":id,
                "error":{"code":-32602,"message":"explicit rollback confirmation is required"}
            });
        }
        let Some(expected_sha) = params
            .get("expected_installed_sha")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        else {
            return json!({
                "jsonrpc":"2.0", "id":id,
                "error":{"code":-32602,"message":"expected_installed_sha is required"}
            });
        };
        match self
            .ports
            .admin
            .rollback_deployment(expected_sha.to_owned())
            .await
        {
            Ok(receipt) => {
                info!(
                    principal = ?connection.principal_id,
                    installed_sha = %receipt.installed_sha,
                    "authenticated deployment rollback completed"
                );
                json!({"jsonrpc":"2.0","id":id,"result":receipt})
            }
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_daemon_shutdown(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.shutdown().await {
            Ok(()) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "status": "shutting_down" }
            }),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_reload_skills(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.reload_skills().await {
            Ok(count) => {
                info!(count, "Skills reloaded via reload_skills RPC");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "skills_loaded": count }
                })
            }
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_skills_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let skills = self.ports.admin.list_skills().await;
        let list: Vec<serde_json::Value> = skills
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "enabled": s.enabled,
                    "extension_id": s.extension_id,
                })
            })
            .collect();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "method": "skills.list",
                "skills": list,
            }
        })
    }

    pub(super) async fn handle_approval_response(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = match serde_json::from_value::<
            ::contracts::protocol::client::ApprovalResponseParams,
        >(request.get("params").cloned().unwrap_or_default())
        {
            Ok(params) => params,
            Err(error) => {
                return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32602,"message":error.to_string()}})
            }
        };
        match self
            .ports
            .admin
            .resolve_transient_approval(TransientApprovalRequest {
                principal_id: connection.principal_id.clone(),
                connection_id: connection.connection_id.clone(),
                approval_id: params.approval_id,
                decision: params.decision,
                scope_hint: params.scope_hint,
            })
            .await
        {
            Ok(_) => json!({"jsonrpc":"2.0", "id":id, "result":{"ok":true}}),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_interrupt(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let reason = match request
            .get("params")
            .and_then(|params| params.get("reason"))
            .and_then(|reason| reason.as_str())
            .unwrap_or("user_cancelled")
        {
            "timeout" => InterruptReason::Timeout,
            "budget_exceeded" => InterruptReason::BudgetExceeded,
            _ => InterruptReason::UserCancelled,
        };
        match self.ports.admin.interrupt(reason).await {
            Ok(()) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "status": "interrupt_requested", "reason": format!("{:?}", reason) }
            }),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_mode_switch(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let mode = match request
            .get("params")
            .and_then(|params| params.get("mode"))
            .and_then(|mode| mode.as_str())
            .unwrap_or("default")
        {
            "plan" => CollaborationMode::Plan,
            "auto" => CollaborationMode::Auto,
            "sandbox" => CollaborationMode::Sandbox,
            _ => CollaborationMode::Default,
        };
        match self.ports.admin.switch_mode(mode).await {
            Ok(change) => {
                if let Some(notify) = &self.notify_tx {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "event",
                        "params": {"type": "mode_changed", "mode": change.new.display_name()}
                    });
                    let _ = notify.send(notification.to_string()).await;
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "status": "mode_switched",
                        "old": change.old.display_name(),
                        "new": change.new.display_name()
                    }
                })
            }
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_model_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.model_catalog().await {
            Ok(catalog) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"models": catalog.models, "current": catalog.current}
            }),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_model_switch(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let model = request["params"]["model"]
            .as_str()
            .unwrap_or("")
            .to_string();
        match self.ports.admin.switch_model(model).await {
            Ok(model) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "status": "ok", "model": model }
            }),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_tools_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.tools().await {
            Ok(tools) => json!({"jsonrpc":"2.0", "id":id, "result":{"tools":tools}}),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_sub_agents(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.sub_agents().await {
            Ok(agents) => json!({"jsonrpc":"2.0", "id":id, "result":{"agents":agents}}),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_agent_profile_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.list_agent_profiles().await {
            Ok(profiles) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"profiles": profiles}
            }),
            Err(error) => admin_error(id, error),
        }
    }

    pub(super) async fn handle_agent_profile_set(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let profile_name = request["params"]["profile"]
            .as_str()
            .unwrap_or("")
            .to_string();
        match self.ports.admin.switch_agent_profile(profile_name).await {
            Ok(result) => {
                if let Some(notify) = &self.notify_tx {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "event",
                        "params": {"type": "profile_changed", "profile": &result.current}
                    });
                    let _ = notify.send(notification.to_string()).await;
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result
                })
            }
            Err(error) => admin_error(id, error),
        }
    }
}

fn admin_error(id: &serde_json::Value, error: AdminServiceError) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32000, "message": error.to_string()}
    })
}
