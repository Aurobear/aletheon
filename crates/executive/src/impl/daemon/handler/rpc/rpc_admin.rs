//! Admin and meta RPC handlers.
//!
//! Methods: daemon.shutdown, reload_skills, approval_response, interrupt,
//! mode_switch, model_list, model_switch, tools/list, hooks_list, sub_agents.

use super::RequestHandler;

use serde_json::json;
use tracing::{info, warn};

use corpus::security::approval::ApprovalDecision;
use fabric::ui_event::{CollaborationMode, InterruptReason};

impl RequestHandler {
    pub(super) async fn handle_daemon_shutdown(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        if let Some(ref token) = self.daemon_cancel_token {
            token.cancel();
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "status": "shutting_down" }
        })
    }

    pub(super) async fn handle_reload_skills(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let count = {
            let mut loader = self.subsystems.corpus.skill_loader.lock().await;
            loader.reload()
        };
        info!(count = count, "Skills reloaded via reload_skills RPC");

        // Rebuild the cached prefix with updated skills.
        // Note: core_memory snapshot is from boot; mid-session memory
        // changes ride the memory_queue, not the prefix.
        {
            let loader = self.subsystems.corpus.skill_loader.lock().await;
            let cm = self.subsystems.memory.core_memory.lock().await;
            let old_prefix = self.subsystems.session.cached_prefix.lock().await;
            let new_prefix = crate::r#impl::daemon::prefix_builder::PrefixBuilder::build(
                &self.subsystems.session.config_prompt,
                loader.skills(),
                &cm,
            );
            if let Some(reason) = crate::r#impl::daemon::prefix_builder::PrefixBuilder::diff_reason(
                &old_prefix,
                &new_prefix,
            ) {
                info!(reason = %reason, "Prefix changed after skill reload (cache will miss)");
            }
            drop(old_prefix);
            drop(cm);
            drop(loader);
            *self.subsystems.session.cached_prefix.lock().await = new_prefix;
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "skills_loaded": count }
        })
    }

    pub(super) async fn handle_approval_response(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        // Resolve a pending approval request. The client sends this
        // in response to an "approval_request" notification.
        // Supports: "once" (approve this time), "always" (approve for session),
        //           "reject" (deny).
        let aid = request["params"]["approval_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let action = request["params"]["decision"]
            .as_str()
            .unwrap_or("reject")
            .to_string();
        let tool_name = request["params"]["tool"].as_str().unwrap_or("").to_string();

        let decision = match action.as_str() {
            "once" => ApprovalDecision::Approve,
            "always" => {
                // Cache approval for this tool for the rest of the session
                if !tool_name.is_empty() {
                    let mut approvals = self.subsystems.security.session_approvals.lock().await;
                    approvals.insert(tool_name.clone(), true);
                    info!(tool = %tool_name, "Tool approved for session (always)");
                }
                ApprovalDecision::ApproveForSession
            }
            _ => ApprovalDecision::Deny,
        };

        if let Some(tx) = self
            .subsystems
            .security
            .pending_approvals
            .lock()
            .await
            .remove(&aid)
        {
            let _ = tx.send(decision);
            info!(approval_id = %aid, action = %action, "Approval resolved");
        } else {
            warn!(approval_id = %aid, "No pending approval found for id");
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "ok": true }
        })
    }

    pub(super) async fn handle_interrupt(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let reason = match request
            .get("params")
            .and_then(|p| p.get("reason"))
            .and_then(|r| r.as_str())
            .unwrap_or("user_cancelled")
        {
            "user_cancelled" => InterruptReason::UserCancelled,
            "timeout" => InterruptReason::Timeout,
            "budget_exceeded" => InterruptReason::BudgetExceeded,
            _ => InterruptReason::UserCancelled,
        };
        {
            self.subsystems
                .runtime
                .lock()
                .await
                .interrupt_flag()
                .request(reason);
        }
        info!(reason = ?reason, "Interrupt requested");
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "status": "interrupt_requested", "reason": format!("{:?}", reason) }
        })
    }

    pub(super) async fn handle_mode_switch(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let mode_str = request
            .get("params")
            .and_then(|p| p.get("mode"))
            .and_then(|m| m.as_str())
            .unwrap_or("default");
        let mode = match mode_str {
            "plan" => CollaborationMode::Plan,
            "auto" => CollaborationMode::Auto,
            "sandbox" => CollaborationMode::Sandbox,
            _ => CollaborationMode::Default,
        };
        let old_mode;
        {
            let mut rt = self.subsystems.runtime.lock().await;
            old_mode = rt.mode_router().current_mode();
            rt.mode_router_mut().set_mode(mode);
        }
        info!(old = ?old_mode, new = ?mode, "Collaboration mode switched");
        // Notify all connected clients about the mode change
        if let Some(ref tx) = self.notify_tx {
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "event",
                "params": {
                    "type": "mode_changed",
                    "mode": mode.display_name(),
                }
            });
            let _ = tx.send(notification.to_string()).await;
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "status": "mode_switched",
                "old": old_mode.display_name(),
                "new": mode.display_name()
            }
        })
    }

    pub(super) async fn handle_model_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "models": [
                    {"name": "default", "description": "Default model from config"},
                    {"name": "sonnet", "description": "Claude Sonnet"},
                    {"name": "opus", "description": "Claude Opus"},
                    {"name": "haiku", "description": "Claude Haiku"}
                ],
                "current": "default"
            }
        })
    }

    pub(super) async fn handle_model_switch(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let model = request["params"]["model"].as_str().unwrap_or("");
        info!(model = %model, "Model switch requested");
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "status": "ok", "model": model }
        })
    }

    pub(super) async fn handle_tools_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let tools_arc = self.subsystems.corpus.tools.clone();
        let reg = tools_arc.lock().await;
        let tools: Vec<serde_json::Value> = reg
            .definitions()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "description": d.description,
                    "input_schema": d.input_schema,
                })
            })
            .collect();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tools }
        })
    }

    pub(super) async fn handle_hooks_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let hr = self.subsystems.corpus.hook_registry.lock().await;
        let hooks: Vec<serde_json::Value> = hr
            .list()
            .iter()
            .map(|h| {
                serde_json::json!({
                    "name": h.name,
                    "source": h.source,
                    "point": format!("{:?}", h.point),
                    "priority": h.priority,
                    "script_path": h.script_path,
                })
            })
            .collect();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "hooks": hooks }
        })
    }

    pub(super) async fn handle_sub_agents(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let agents: Vec<_> = self
            .subsystems
            .runtime
            .lock()
            .await
            .sub_agent_spawner()
            .list()
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "task": a.task,
                    "status": format!("{:?}", a.status),
                })
            })
            .collect();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "agents": agents }
        })
    }
}
