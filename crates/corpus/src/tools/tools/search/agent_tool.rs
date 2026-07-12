//! AgentTool — delegates tasks to sub-agents with independent execution contexts.
//!
//! Each sub-agent runs with its own tool pool and system prompt as defined
//! in the agent's markdown definition file.

use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// Agent definition loaded from markdown files.
/// This mirrors the runtime's AgentDefinition to avoid cross-crate deps.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub system_prompt: String,
}

/// Function type for executing a sub-agent turn.
/// Takes (system_prompt, user_prompt, allowed_tool_names) and returns the response.
pub type ExecuteSubAgentFn = Arc<
    dyn Fn(String, String, Vec<String>) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
        + Send
        + Sync,
>;

/// A tool that delegates tasks to sub-agents.
///
/// Each sub-agent runs with its own tool pool and system prompt as defined
/// in the agent's markdown definition file. The actual LLM execution is
/// delegated to a callback function provided at construction time.
pub struct AgentTool {
    agents: HashMap<String, AgentDefinition>,
    execute_fn: ExecuteSubAgentFn,
}

impl AgentTool {
    pub fn new(agents: HashMap<String, AgentDefinition>, execute_fn: ExecuteSubAgentFn) -> Self {
        Self { agents, execute_fn }
    }

    /// Get list of available agent names.
    fn agent_names(&self) -> Vec<&str> {
        self.agents.keys().map(|s| s.as_str()).collect()
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Delegate a task to a specialized sub-agent with its own tools and system prompt"
    }

    fn input_schema(&self) -> serde_json::Value {
        let agent_names: Vec<&str> = self.agent_names();
        json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "description": "The type of agent to delegate to",
                    "enum": agent_names
                },
                "prompt": {
                    "type": "string",
                    "description": "The task or question to delegate to the sub-agent"
                }
            },
            "required": ["agent_type", "prompt"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(AgentTool {
            agents: self.agents.clone(),
            execute_fn: self.execute_fn.clone(),
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let agent_type = input["agent_type"].as_str().unwrap_or("");
        let prompt = input["prompt"].as_str().unwrap_or("");

        if agent_type.is_empty() || prompt.is_empty() {
            return ToolResult {
                content: "Both 'agent_type' and 'prompt' are required".to_string(),
                is_error: true,
                metadata: ToolResultMeta::default(),
            };
        }

        // Look up agent definition
        let agent_def = match self.agents.get(agent_type) {
            Some(def) => def,
            None => {
                let available: Vec<&str> = self.agent_names();
                return ToolResult {
                    content: format!(
                        "Unknown agent type: '{}'. Available agents: {:?}",
                        agent_type, available
                    ),
                    is_error: true,
                    metadata: ToolResultMeta::default(),
                };
            }
        };

        // Build system prompt with delegation context
        let system_prompt = format!(
            "{}\n\n---\nYou are a sub-agent delegated by the main agent. \
             Focus on completing the assigned task efficiently.",
            agent_def.system_prompt
        );

        // Get the list of allowed tools for this agent
        let allowed_tools = agent_def.tools.clone();

        // Execute the sub-agent via the callback
        let result = (self.execute_fn)(system_prompt, prompt.to_string(), allowed_tools).await;

        match result {
            Ok(response) => ToolResult {
                content: response,
                is_error: false,
                metadata: ToolResultMeta::default(),
            },
            Err(e) => ToolResult {
                content: format!("Sub-agent error: {}", e),
                is_error: true,
                metadata: ToolResultMeta::default(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_tool_schema() {
        let mut agents = HashMap::new();
        agents.insert(
            "test-agent".to_string(),
            AgentDefinition {
                name: "test-agent".to_string(),
                description: "Test agent".to_string(),
                tools: vec!["bash_exec".to_string()],
                model: None,
                max_iterations: 20,
                system_prompt: "You are a test agent.".to_string(),
            },
        );

        let execute_fn: ExecuteSubAgentFn = Arc::new(|_, _, _| {
            Box::pin(async { Ok("test response".to_string()) })
        });

        let tool = AgentTool::new(agents, execute_fn);
        assert_eq!(tool.name(), "agent");
        assert_eq!(tool.permission_level(), PermissionLevel::L1);

        let schema = tool.input_schema();
        assert!(schema["properties"]["agent_type"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("test-agent")));
    }
}
