use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

use super::agent::{Agent, Capability};
use corpus::tools::tools::Tool;
use fabric::message::{ContentBlock, Message};
use fabric::{LlmProvider, ToolDefinition};

/// TOML agent configuration (loaded from `agents/<id>.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFileConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
    pub role: AgentRole,
    pub max_iterations: usize,
    /// Filename of the system prompt markdown, relative to the agent directory.
    pub system_prompt: String,
}

/// Agent role — controls delegation behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    /// Cannot delegate; focused worker.
    Leaf,
    /// Can delegate to other agents, depth-limited.
    Orchestrator,
}

/// An agent loaded from TOML + markdown config files.
///
/// Behavior is driven by the system prompt (loaded from a `.md` file),
/// not by code. Tools are filtered to only those listed in the TOML config.
pub struct ConfigAgent {
    file_config: AgentFileConfig,
    system_prompt_text: String,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Capability>,
    llm: Box<dyn LlmProvider>,
}

impl ConfigAgent {
    /// Load a `ConfigAgent` from a TOML file path.
    ///
    /// `all_tools` is the full set of available tools; only those whose names
    /// appear in the TOML `tools` list are kept.
    pub fn load(
        toml_path: &Path,
        all_tools: &[Box<dyn Tool>],
        llm: Box<dyn LlmProvider>,
    ) -> Result<Self> {
        let content = std::fs::read_to_string(toml_path)?;
        let wrapper: AgentFileConfigWrapper = toml::from_str(&content)?;
        let file_config = wrapper.agent;

        let agent_dir = toml_path.parent().unwrap();
        let prompt_path = agent_dir.join(&file_config.system_prompt);
        let system_prompt_text = if prompt_path.exists() {
            std::fs::read_to_string(&prompt_path)?
        } else {
            tracing::warn!(
                path = %prompt_path.display(),
                "System prompt file not found, using fallback"
            );
            format!("You are {}.", file_config.name)
        };

        // Filter tools to only those declared in config
        let tools: Vec<Box<dyn Tool>> = all_tools
            .iter()
            .filter(|t| file_config.tools.contains(&t.name().to_string()))
            .map(|t| t.boxed_clone())
            .collect();

        // Build Capability structs from config strings
        let capabilities: Vec<Capability> = file_config
            .capabilities
            .iter()
            .map(|name| Capability {
                name: name.clone(),
                description: String::new(),
            })
            .collect();

        info!(
            id = %file_config.id,
            tools = tools.len(),
            capabilities = capabilities.len(),
            role = ?file_config.role,
            "Loaded agent from config"
        );

        Ok(Self {
            file_config,
            system_prompt_text,
            tools,
            capabilities,
            llm,
        })
    }

    /// Whether this agent can delegate to other agents.
    pub fn can_delegate(&self) -> bool {
        self.file_config.role == AgentRole::Orchestrator
    }
}

/// Wrapper for TOML deserialization (`[agent]` table).
#[derive(Deserialize)]
struct AgentFileConfigWrapper {
    agent: AgentFileConfig,
}

#[async_trait]
impl Agent for ConfigAgent {
    fn id(&self) -> &str {
        &self.file_config.id
    }

    fn name(&self) -> &str {
        &self.file_config.name
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    fn tools(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }

    fn system_prompt(&self) -> Option<&str> {
        Some(&self.system_prompt_text)
    }

    fn description(&self) -> String {
        self.file_config.description.clone()
    }

    async fn handle_task(&self, task: &str) -> Result<String> {
        // Build messages: system prompt + user task
        let mut messages = Vec::new();
        messages.push(Message::system(&self.system_prompt_text));
        messages.push(Message::user(task));

        // Build tool definitions for the LLM
        let tool_defs: Vec<ToolDefinition> = self
            .tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();

        // Simple ReAct loop scoped to this agent
        for _iteration in 0..self.file_config.max_iterations {
            let response = self.llm.complete(&messages, &tool_defs).await?;

            // Collect tool use blocks
            let tool_calls: Vec<(&str, &str, &serde_json::Value)> = response
                .content
                .iter()
                .filter_map(|c| match c {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.as_str(), name.as_str(), input))
                    }
                    _ => None,
                })
                .collect();

            if tool_calls.is_empty() {
                // Final text response
                let text = response
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(text);
            }

            // Append assistant message with tool uses
            messages.push(Message {
                role: fabric::Role::Assistant,
                content: response.content.clone(),
            });

            // Execute each tool call and append results
            for (id, name, input) in &tool_calls {
                let tool = self.tools.iter().find(|t| t.name() == *name);
                let result_text = if let Some(tool) = tool {
                    let ctx = fabric::tool::ToolContext {
                        working_dir: std::path::PathBuf::from("."),
                        session_id: String::new(),
                    };
                    let result = tool.execute((*input).clone(), &ctx).await;
                    result.content
                } else {
                    format!("Error: tool '{}' not available to this agent", name)
                };

                messages.push(Message::tool_result(id.to_string(), result_text, false));
            }
        }

        Ok("Agent reached maximum iterations without a final response.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agent_toml() {
        let toml_str = r#"
[agent]
id = "test-agent"
name = "Test Agent"
description = "A test agent"
capabilities = ["testing"]
tools = ["file_read"]
role = "Leaf"
max_iterations = 5
system_prompt = "test-agent.md"
"#;
        let wrapper: AgentFileConfigWrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.agent.id, "test-agent");
        assert_eq!(wrapper.agent.role, AgentRole::Leaf);
        assert_eq!(wrapper.agent.tools, vec!["file_read"]);
    }

    #[test]
    fn test_parse_orchestrator_role() {
        let toml_str = r#"
[agent]
id = "orch"
name = "Orchestrator"
description = "Coordinates agents"
capabilities = ["delegation"]
tools = ["delegate_task"]
role = "Orchestrator"
max_iterations = 50
system_prompt = "orch.md"
"#;
        let wrapper: AgentFileConfigWrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(wrapper.agent.role, AgentRole::Orchestrator);
    }
}
