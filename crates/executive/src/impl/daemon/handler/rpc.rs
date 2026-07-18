//! JSON-RPC method dispatcher.
//!
//! Routes incoming RPC methods to the appropriate handler sub-module.
//! Each sub-module is responsible for a single logical group of methods.

mod rpc_admin;
mod rpc_approval;
mod rpc_goal;
mod rpc_google;
mod rpc_health;
mod rpc_memory;
mod rpc_reflection;
mod rpc_session;
mod rpc_turn;
mod rpc_workflow;

use super::RequestHandler;

impl RequestHandler {
    /// Dispatch a non-chat, non-session-gateway, non-debug RPC method to the
    /// appropriate handler sub-module.  Each arm delegates to a focused handler
    /// function defined in one of the `rpc_*` sub-modules.
    pub(super) async fn handle_rpc(
        &self,
        connection: &super::super::server::ConnectionContext,
        method: &str,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        match method {
            // ── Session lifecycle ──────────────────────────────────────
            "clear" => self.handle_clear(&id, &request).await,
            "sessions" => self.handle_sessions_list(&id, &request).await,
            "resume" => self.handle_resume(&id, &request).await,
            "compact" => self.handle_compact(&id, &request).await,
            "new_session" => self.handle_new_session(&id, &request).await,
            "load_recent" => self.handle_load_recent(&id, &request).await,
            "session.create" => self.handle_session_create(&id, &request).await,
            "session.list" => self.handle_session_list(&id, &request).await,
            "session.switch" => self.handle_session_switch(&id, &request).await,

            // ── Health / status ───────────────────────────────────────
            "status" => self.handle_status(&id, &request).await,
            "health" => self.handle_health(&id, &request).await,

            // ── Admin / meta ──────────────────────────────────────────
            "daemon.shutdown" => self.handle_daemon_shutdown(&id, &request).await,
            "reload_skills" => self.handle_reload_skills(&id, &request).await,
            "approval_response" => {
                self.handle_approval_response(connection, &id, &request)
                    .await
            }
            "approval.list" => self.handle_approval_list(connection, &id, &request).await,
            "approval.show" => self.handle_approval_show(connection, &id, &request).await,
            "approval.approve" => {
                self.handle_approval_approve(connection, &id, &request)
                    .await
            }
            "approval.reject" => self.handle_approval_reject(connection, &id, &request).await,
            "interrupt" => self.handle_interrupt(&id, &request).await,
            "mode_switch" => self.handle_mode_switch(&id, &request).await,
            "model_list" => self.handle_model_list(&id, &request).await,
            "model_switch" => self.handle_model_switch(&id, &request).await,
            "tools/list" => self.handle_tools_list(&id, &request).await,
            "google.authorization.start" => {
                self.handle_google_authorization_start(&id, &request).await
            }
            "google.authorization.callback" => {
                self.handle_google_authorization_callback(&id, &request)
                    .await
            }
            "google.accounts.list" => self.handle_google_accounts_list(&id, &request).await,
            "google.accounts.revoke" => self.handle_google_account_revoke(&id, &request).await,
            "google.token.refresh" => self.handle_google_token_refresh(&id, &request).await,
            "hooks_list" => self.handle_hooks_list(&id, &request).await,
            "sub_agents" => self.handle_sub_agents(&id, &request).await,
            "agent.profile.list" => self.handle_agent_profile_list(&id, &request).await,
            "agent.profile.set" => self.handle_agent_profile_set(&id, &request).await,

            // ── Reflection / self-awareness ───────────────────────────
            "reflect" => self.handle_reflect(&id, &request).await,
            "reflect_now" => self.handle_reflect_now(&id, &request).await,
            "genome" => self.handle_genome(&id, &request).await,
            "evolution" => self.handle_evolution(&id, &request).await,

            // ── Memory (fact store) ───────────────────────────────────
            "memory.add" => self.handle_memory_add(&id, &request).await,
            "memory.list" => self.handle_memory_list(&id, &request).await,
            "memory.search" => self.handle_memory_search(&id, &request).await,
            "memory.show" => self.handle_memory_show(&id, &request).await,
            "memory.forget" => self.handle_memory_forget(&id, &request).await,
            "memory.pin" | "memory.unpin" => self.handle_memory_pin(&id, &request, method).await,

            // ── Workflow persistence / execution ──────────────────────
            "workflow.save" => self.handle_workflow_save(&id, &request).await,
            "workflow.load" => self.handle_workflow_load(&id, &request).await,
            "workflow.list" => self.handle_workflow_list(&id, &request).await,
            "workflow.delete" => self.handle_workflow_delete(&id, &request).await,
            "workflow.run" => self.handle_workflow_run(&id, &request).await,

            // ── Goal / objective tracking ─────────────────────────────
            "goal.set" => self.handle_goal_set(&id, &request).await,
            "goal.show" => self.handle_goal_show(&id, &request).await,
            "goal.status" => self.handle_goal_status(&id, &request).await,
            "goal.resume" => self.handle_goal_resume(&id, &request).await,
            "goal.create" => self.handle_goal_create(&id, &request).await,
            "goal.list" => self.handle_goal_list(&id, &request).await,
            "goal.pause" => self.handle_goal_pause(&id, &request).await,
            "goal.run" => self.handle_goal_run(&id, &request).await,
            "goal.cancel" => self.handle_goal_cancel(&id, &request).await,

            // ── Turn lifecycle (PR-3) ─────────────────────────────────
            "turn.wait" => self.handle_turn_wait(&id, &request).await,
            "turn.cancel" => self.handle_turn_cancel(connection, &id, &request).await,
            "turn.exit" => self.handle_turn_exit(&id, &request).await,
            "prompt.edit" if self.grok_hardening.prompt_queue => {
                self.handle_prompt_edit(connection, &id, &request).await
            }
            "prompt.cancel" if self.grok_hardening.prompt_queue => {
                self.handle_prompt_cancel(connection, &id, &request).await
            }
            "prompt.metrics" if self.grok_hardening.prompt_queue => {
                self.handle_prompt_metrics(connection, &id, &request).await
            }
            "workspace.rewind" if self.grok_hardening.workspace_checkpoint => {
                self.handle_workspace_rewind(connection, &id, &request)
                    .await
            }

            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}
