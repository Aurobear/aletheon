//! Compatibility `agent` tool implemented as bounded Agent control spawn + wait.

use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use ::contracts::{
    AgentBudget, AgentContextFork, AgentControlPort, AgentProfile, AgentSpawnRequest,
    AgentWaitRequest, RuntimeId,
};

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

pub struct AgentTool {
    profiles: HashMap<String, AgentProfile>,
    control: Arc<dyn AgentControlPort>,
    runtime_id: RuntimeId,
    wait_timeout_ms: u64,
}

impl AgentTool {
    pub fn new(
        profiles: HashMap<String, AgentProfile>,
        control: Arc<dyn AgentControlPort>,
        runtime_id: RuntimeId,
    ) -> Self {
        Self {
            profiles,
            control,
            runtime_id,
            wait_timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        }
    }

    pub fn with_wait_timeout(mut self, timeout_ms: u64) -> Self {
        self.wait_timeout_ms = timeout_ms.max(1);
        self
    }

    fn agent_names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Delegate a task to a configured child Agent and wait for its bounded result"
    }

    fn input_schema(&self) -> serde_json::Value {
        let agent_names = self.agent_names();
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agent_type": {
                    "type": "string",
                    "enum": agent_names
                },
                "prompt": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 65536
                }
            },
            "required": ["agent_type", "prompt"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self {
            profiles: self.profiles.clone(),
            control: self.control.clone(),
            runtime_id: self.runtime_id.clone(),
            wait_timeout_ms: self.wait_timeout_ms,
        })
    }

    async fn execute(&self, input: serde_json::Value, context: &ToolContext) -> ToolResult {
        let Some(trusted) = context.agent.as_ref() else {
            return tool_error("Agent tool requires trusted lifecycle context");
        };
        let Some(agent_type) = input.get("agent_type").and_then(|value| value.as_str()) else {
            return tool_error("Both 'agent_type' and 'prompt' are required");
        };
        let Some(prompt) = input.get("prompt").and_then(|value| value.as_str()) else {
            return tool_error("Both 'agent_type' and 'prompt' are required");
        };
        let Some(profile) = self.profiles.get(agent_type) else {
            return tool_error("Unknown Agent profile");
        };
        let max_depth = trusted
            .delegator_authority
            .as_ref()
            .map(|authority| authority.budget.max_depth)
            .unwrap_or(1);
        let request = AgentSpawnRequest {
            root_agent_id: trusted.caller_root_agent_id,
            parent_agent_id: Some(trusted.parent_agent_id),
            parent_process_id: Some(trusted.parent_process_id),
            profile_id: profile.id.clone(),
            runtime_id: self.runtime_id.clone(),
            trusted_workspace: match context.effective_workspace_policy() {
                Ok(workspace) => Some(workspace),
                Err(error) => return tool_error(&format!("Invalid Agent workspace: {error}")),
            },
            delegator_authority: trusted.delegator_authority.clone(),
            cognitive_binding: None,
            task: prompt.to_string(),
            context: AgentContextFork::None,
            broadcast_refs: vec![],
            allowed_tools: profile.allowed_tools.clone(),
            background_decls: vec![],
            budget: AgentBudget {
                max_input_tokens: profile.max_input_tokens,
                max_output_tokens: profile.max_output_tokens,
                max_tool_calls: profile.max_tool_calls,
                max_elapsed_ms: profile.max_elapsed_ms,
                max_cost_usd: None,
                max_depth,
            },
        };
        if let Err(error) = request.validate() {
            return tool_error(&format!("Invalid Agent request: {:?}", error.kind));
        }
        let handle = match self.control.spawn(request).await {
            Ok(handle) => handle,
            Err(error) => return tool_error(&format!("Agent spawn failed: {:?}", error.kind)),
        };
        let snapshot = match self
            .control
            .wait(AgentWaitRequest {
                caller_root_agent_id: trusted.caller_root_agent_id,
                agent_id: handle.agent_id,
                timeout_ms: self.wait_timeout_ms,
            })
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => return tool_error(&format!("Agent wait failed: {:?}", error.kind)),
        };
        match snapshot.result {
            Some(result) if snapshot.status == ::contracts::AgentRunStatus::Succeeded => {
                ToolResult {
                    content: result.output,
                    is_error: false,
                    metadata: ToolResultMeta::default(),
                }
            }
            _ => tool_error(&format!(
                "Agent terminated with status {:?}",
                snapshot.status
            )),
        }
    }
}

fn tool_error(message: &str) -> ToolResult {
    ToolResult {
        content: message.to_string(),
        is_error: true,
        metadata: ToolResultMeta::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_bounded_and_has_no_identity_fields() {
        let tool = AgentTool::new(
            HashMap::new(),
            Arc::new(TestControl),
            RuntimeId("native-cognit".into()),
        );
        let schema = tool.input_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("caller_root_agent_id").is_none());
    }

    struct TestControl;

    #[async_trait]
    impl AgentControlPort for TestControl {
        async fn spawn(
            &self,
            _request: AgentSpawnRequest,
        ) -> Result<::contracts::AgentHandle, ::contracts::AgentControlError> {
            unreachable!()
        }
        async fn wait(
            &self,
            _request: AgentWaitRequest,
        ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
            unreachable!()
        }
        async fn send(
            &self,
            _request: ::contracts::AgentSendRequest,
        ) -> Result<::contracts::AgentControlMessage, ::contracts::AgentControlError> {
            unreachable!()
        }
        async fn cancel(
            &self,
            _caller_root_agent_id: ::contracts::AgentId,
            _agent_id: ::contracts::AgentId,
        ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
            unreachable!()
        }
        async fn inspect(
            &self,
            _caller_root_agent_id: ::contracts::AgentId,
            _agent_id: ::contracts::AgentId,
        ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
            unreachable!()
        }
        async fn list(
            &self,
            _request: ::contracts::AgentListRequest,
        ) -> Result<Vec<::contracts::AgentSnapshot>, ::contracts::AgentControlError> {
            unreachable!()
        }
    }
}
