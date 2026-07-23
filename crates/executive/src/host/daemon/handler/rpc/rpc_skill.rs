//! Daemon-authoritative Skill command invocation.

use serde_json::json;

use super::super::{resolve_requested_workspace, RequestHandler};

impl RequestHandler {
    pub(super) async fn handle_skill_invoke(
        &self,
        connection: &crate::host::daemon::server::ConnectionContext,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let skill_id = request["params"]["skill_id"].as_str().unwrap_or("").trim();
        if skill_id.is_empty() {
            return json!({"jsonrpc":"2.0","id":id,"error":{
                "code":-32602,
                "message":"skill_id is required",
                "data":{"recovery_hint":"Choose a command from /skills","retryable":true}
            }});
        }
        let descriptor = self
            .ports
            .admin
            .list_skills()
            .await
            .into_iter()
            .find(|skill| skill.enabled && skill.id == skill_id);
        let Some(descriptor) = descriptor else {
            return json!({"jsonrpc":"2.0","id":id,"error":{
                "code":-32044,
                "message":format!("Skill is unavailable or disabled: {skill_id}"),
                "data":{"recovery_hint":"Refresh /skills and retry with an enabled Skill","retryable":true}
            }});
        };
        let workspace = match resolve_requested_workspace(&request["params"]) {
            Ok(workspace) => workspace,
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{
                    "code":-32602,"message":error,
                    "data":{"recovery_hint":"Invoke the Skill from a valid workspace","retryable":false}
                }});
            }
        };
        let user_args = request["params"]["user_args"].as_str().unwrap_or("").trim();
        // Skill definitions are already loaded by the daemon into the trusted
        // cache-stable prefix. This daemon-generated selector binds one of those
        // definitions to user arguments without letting the TUI inject SKILL.md.
        let message = if user_args.is_empty() {
            format!("Invoke the registered Skill `{}` now.", descriptor.name)
        } else {
            format!(
                "Invoke the registered Skill `{}` now.\n\nUser input:\n{}",
                descriptor.name, user_args
            )
        };
        let thread_id = match self.select_workspace_session(workspace.cwd()).await {
            Ok(thread_id) => thread_id,
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{
                    "code":-32603,"message":error.to_string(),
                    "data":{"recovery_hint":"Retry after the session is available","retryable":true}
                }});
            }
        };
        self.execute_explicit_chat(connection, id, message, thread_id, workspace)
            .await
    }
}
