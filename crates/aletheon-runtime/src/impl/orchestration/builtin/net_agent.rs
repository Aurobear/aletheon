use async_trait::async_trait;

use aletheon_abi::message::{ContentBlock, Message};
use aletheon_brain::r#impl::llm::{LlmProvider, ToolDefinition};
use aletheon_body::r#impl::tools::system_status::SystemStatusTool;
use aletheon_body::r#impl::tools::process_list::ProcessListTool;
use aletheon_body::r#impl::tools::Tool;
use super::super::agent::{Agent, AgentConfig, Capability};

/// Network agent — handles network operations and monitoring.
pub struct NetAgent {
    config: AgentConfig,
    llm: Box<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Capability>,
}

impl NetAgent {
    pub fn new(llm: Box<dyn LlmProvider>) -> Self {
        let config = AgentConfig {
            id: "net_agent".to_string(),
            name: "Network Agent".to_string(),
            system_prompt: Some(
                "You are a network agent. You can monitor network status and manage services."
                    .to_string(),
            ),
        };

        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(SystemStatusTool),
            Box::new(ProcessListTool),
        ];

        let capabilities = vec![
            Capability {
                name: "network_info".to_string(),
                description: "Get network and system status".to_string(),
            },
            Capability {
                name: "service_control".to_string(),
                description: "List and monitor running processes".to_string(),
            },
        ];

        Self { config, llm, tools, capabilities }
    }
}

#[async_trait]
impl Agent for NetAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    fn tools(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }

    fn system_prompt(&self) -> Option<&str> {
        self.config.system_prompt.as_deref()
    }

    async fn handle_task(&self, task: &str) -> anyhow::Result<String> {
        let messages = vec![Message::user(task)];
        let tool_defs: Vec<ToolDefinition> = self
            .tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();

        let response = self.llm.complete(&messages, &tool_defs).await?;
        let content = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(content)
    }
}
