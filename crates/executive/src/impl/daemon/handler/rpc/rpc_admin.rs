//! Admin and meta RPC handlers.

use super::RequestHandler;
use crate::service::admin_service::{AdminServiceError, TransientApprovalRequest};
use fabric::ui_event::{CollaborationMode, InterruptReason};
use serde_json::json;
use tracing::info;

impl RequestHandler {
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

    pub(super) async fn handle_approval_response(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let approval_id = request["params"]["approval_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let decision = request["params"]["decision"]
            .as_str()
            .unwrap_or("reject")
            .to_string();
        let tool_name = request["params"]["tool"].as_str().unwrap_or("").to_string();
        match self
            .ports
            .admin
            .resolve_transient_approval(TransientApprovalRequest {
                approval_id,
                decision,
                tool_name,
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

    pub(super) async fn handle_hooks_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.admin.hooks().await {
            Ok(hooks) => json!({"jsonrpc":"2.0", "id":id, "result":{"hooks":hooks}}),
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
}

fn admin_error(id: &serde_json::Value, error: AdminServiceError) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32000, "message": error.to_string()}
    })
}
