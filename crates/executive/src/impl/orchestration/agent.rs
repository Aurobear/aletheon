use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use corpus::tools::tools::Tool;
use fabric::message::{ContentBlock, Message};

/// A capability that an agent advertises.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Human-readable capability name (e.g. "code_review", "file_edit").
    pub name: String,
    /// Optional description.
    pub description: String,
}

/// Configuration for constructing an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique agent identifier.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional system prompt / persona description.
    pub system_prompt: Option<String>,
}

/// Canonical Agent trait for multi-agent orchestration.
///
/// DEPRECATED: Use `AgentProcess` + `AgentKernel` for agent lifecycle management.
/// This trait remains for backward compatibility with existing orchestration code.
///
/// Each agent has an identity, a set of capabilities it advertises,
/// and a set of tools it can use. The orchestration layer uses this
/// trait to discover, route, and coordinate agents.
#[async_trait]
pub trait Agent: Send + Sync {
    /// Unique identifier for this agent.
    fn id(&self) -> &str;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Capabilities this agent advertises.
    fn capabilities(&self) -> &[Capability];

    /// Tools available to this agent.
    fn tools(&self) -> &[Box<dyn Tool>];

    /// Optional system prompt / persona.
    fn system_prompt(&self) -> Option<&str>;

    /// Handle a task dispatched to this agent. Returns a textual result.
    async fn handle_task(&self, task: &str) -> anyhow::Result<String>;

    /// Handle a structured message and return an AgentResponse.
    ///
    /// Default implementation wraps `handle_task` into an `AgentResponse`.
    async fn on_message(&self, message: Message) -> anyhow::Result<AgentResponse> {
        let text = message
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let result = self.handle_task(text).await?;
        Ok(AgentResponse {
            content: result,
            tool_calls_made: 0,
            iterations_used: 1,
            status: AgentResponseStatus::Completed,
        })
    }

    /// Short description for routing purposes.
    fn description(&self) -> String {
        self.system_prompt().unwrap_or(self.name()).to_string()
    }
}

/// Status of an agent response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentResponseStatus {
    /// Agent completed the task normally.
    Completed,
    /// Agent requests handoff to another agent.
    HandoffRequested,
    /// Agent hit the iteration limit.
    MaxIterationsReached,
    /// Agent encountered an error.
    Error,
}

/// Structured response from an agent.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// Textual content of the response.
    pub content: String,
    /// Number of tool calls made during this response.
    pub tool_calls_made: usize,
    /// Number of iterations consumed.
    pub iterations_used: usize,
    /// Final status.
    pub status: AgentResponseStatus,
}
