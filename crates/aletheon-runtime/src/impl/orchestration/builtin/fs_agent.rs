use async_trait::async_trait;

use aletheon_abi::message::{ContentBlock, Message};
use aletheon_brain::r#impl::llm::{LlmProvider, ToolDefinition};
use aletheon_body::r#impl::tools::file_read::FileReadTool;
use aletheon_body::r#impl::tools::file_write::FileWriteTool;
use aletheon_body::r#impl::tools::Tool;
use super::super::agent::{Agent, AgentConfig, Capability};

/// File system agent — handles file operations.
pub struct FsAgent {
    config: AgentConfig,
    llm: Box<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    capabilities: Vec<Capability>,
}

impl FsAgent {
    pub fn new(llm: Box<dyn LlmProvider>) -> Self {
        let config = AgentConfig {
            id: "fs_agent".to_string(),
            name: "File System Agent".to_string(),
            system_prompt: Some(
                "You are a file system agent. You can read, write, and manage files.".to_string(),
            ),
        };

        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(FileReadTool),
            Box::new(FileWriteTool),
        ];

        let capabilities = vec![
            Capability {
                name: "file_operations".to_string(),
                description: "Read and write files".to_string(),
            },
        ];

        Self { config, llm, tools, capabilities }
    }
}

#[async_trait]
impl Agent for FsAgent {
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
