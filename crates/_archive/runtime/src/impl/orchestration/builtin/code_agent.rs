use async_trait::async_trait;

use super::super::agent::{Agent, AgentConfig, Capability};
use base::message::{ContentBlock, Message};
use corpus::tools::tools::bash_exec::BashExecTool;
use corpus::tools::tools::file_read::FileReadTool;
use corpus::tools::tools::file_write::FileWriteTool;
use corpus::tools::tools::Tool;
use cognit::r#impl::llm::{LlmProvider, ToolDefinition};

/// Code agent — handles code generation and editing.
pub struct CodeAgent {
    config: AgentConfig,
    llm: Box<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Capability>,
}

impl CodeAgent {
    pub fn new(llm: Box<dyn LlmProvider>) -> Self {
        let config = AgentConfig {
            id: "code_agent".to_string(),
            name: "Code Agent".to_string(),
            system_prompt: Some(
                "You are a code agent. You can read, write, and execute code.".to_string(),
            ),
        };

        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(FileReadTool),
            Box::new(FileWriteTool),
            Box::new(BashExecTool),
        ];

        let capabilities = vec![
            Capability {
                name: "code_generation".to_string(),
                description: "Generate code files".to_string(),
            },
            Capability {
                name: "code_editing".to_string(),
                description: "Edit existing code and run scripts".to_string(),
            },
        ];

        Self {
            config,
            llm,
            tools,
            capabilities,
        }
    }
}

#[async_trait]
impl Agent for CodeAgent {
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
