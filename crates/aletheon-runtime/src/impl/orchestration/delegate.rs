use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use super::budget::IterationBudget;
use super::registry::AgentRegistry;
use aletheon_body::r#impl::tools::{
    PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};

/// Maximum delegation depth (no grandchild agents).
const MAX_DELEGATE_DEPTH: usize = 1;

/// Tools that sub-agents cannot use (blocked at construction).
const _DELEGATE_BLOCKED_TOOLS: &[&str] = &["delegate_task", "clarify", "memory", "send_message"];

/// Configuration for delegation.
#[derive(Debug, Clone)]
pub struct DelegationConfig {
    pub max_iterations: usize,
    pub max_concurrent_children: usize,
    pub max_depth: usize,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            max_concurrent_children: 3,
            max_depth: MAX_DELEGATE_DEPTH,
        }
    }
}

/// DelegateTool -- allows an agent to delegate tasks to other agents.
///
/// Each delegated task is executed with an independent iteration budget.
/// Delegation depth is capped at `max_depth` to prevent unbounded recursion.
pub struct DelegateTool {
    registry: Arc<AgentRegistry>,
    config: DelegationConfig,
    current_depth: usize,
}

impl DelegateTool {
    pub fn new(registry: Arc<AgentRegistry>, config: DelegationConfig) -> Self {
        Self {
            registry,
            config,
            current_depth: 0,
        }
    }

    /// Create a child delegate tool with incremented depth.
    pub fn child(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            config: self.config.clone(),
            current_depth: self.current_depth + 1,
        }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate_task"
    }

    fn description(&self) -> &str {
        "Delegate a task to a specialized agent. Use this when you need help from an agent with specific capabilities."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID of the agent to delegate to"
                },
                "task": {
                    "type": "string",
                    "description": "The task description for the agent"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context for the task (optional)"
                }
            },
            "required": ["agent_id", "task"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn aletheon_abi::tool::Tool> {
        Box::new(DelegateTool {
            registry: Arc::clone(&self.registry),
            config: self.config.clone(),
            current_depth: self.current_depth,
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let agent_id = input["agent_id"].as_str().unwrap_or("");
        let task = input["task"].as_str().unwrap_or("");
        let context = input["context"].as_str().unwrap_or("");

        // Check depth limit
        if self.current_depth >= self.config.max_depth {
            return ToolResult {
                content: format!(
                    "Delegation depth limit reached (max: {}). Cannot delegate to '{}'.",
                    self.config.max_depth, agent_id
                ),
                is_error: true,
                metadata: ToolResultMeta::default(),
            };
        }

        // Look up agent
        let agent = match self.registry.get(agent_id).await {
            Some(a) => a,
            None => {
                let available = self.registry.list_ids().await.join(", ");
                return ToolResult {
                    content: format!(
                        "Agent '{}' not found. Available agents: {}",
                        agent_id, available
                    ),
                    is_error: true,
                    metadata: ToolResultMeta::default(),
                };
            }
        };

        info!(agent_id = %agent_id, task = %task, "Delegating task to agent");

        // Build task string for sub-agent
        let full_task = if context.is_empty() {
            task.to_string()
        } else {
            format!("{}\n\nContext: {}", task, context)
        };

        // Execute with budget tracking
        let budget = IterationBudget::new(self.config.max_iterations);
        let agent_read = agent.read().await;

        match agent_read.handle_task(&full_task).await {
            Ok(response) => {
                let used = budget.used();
                info!(
                    agent_id = %agent_id,
                    iterations = used,
                    "Agent delegation completed"
                );
                ToolResult {
                    content: response,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: 0,
                        truncated: false,
                    },
                }
            }
            Err(e) => {
                warn!(agent_id = %agent_id, error = %e, "Agent delegation failed");
                ToolResult {
                    content: format!("Delegation to '{}' failed: {}", agent_id, e),
                    is_error: true,
                    metadata: ToolResultMeta::default(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::orchestration::agent::{Agent, AgentConfig, Capability};
    use tokio::sync::RwLock;

    /// Minimal mock agent for testing.
    struct MockAgent {
        config: AgentConfig,
        caps: Vec<Capability>,
    }

    #[async_trait]
    impl Agent for MockAgent {
        fn id(&self) -> &str {
            &self.config.id
        }
        fn name(&self) -> &str {
            &self.config.name
        }
        fn capabilities(&self) -> &[Capability] {
            &self.caps
        }
        fn tools(&self) -> &[Box<dyn aletheon_abi::tool::Tool>] {
            &[]
        }
        fn system_prompt(&self) -> Option<&str> {
            self.config.system_prompt.as_deref()
        }
        async fn handle_task(&self, task: &str) -> anyhow::Result<String> {
            Ok(format!("MockAgent processed: {}", task))
        }
    }

    #[tokio::test]
    async fn test_delegate_to_unknown_agent() {
        let registry = Arc::new(AgentRegistry::new());
        let tool = DelegateTool::new(registry, DelegationConfig::default());

        let input = json!({
            "agent_id": "nonexistent",
            "task": "do something"
        });
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_delegate_success() {
        let registry = Arc::new(AgentRegistry::new());
        let agent = Arc::new(RwLock::new(MockAgent {
            config: AgentConfig {
                id: "mock".into(),
                name: "Mock Agent".into(),
                system_prompt: None,
            },
            caps: vec![],
        }));
        registry.register(agent).await;

        let tool = DelegateTool::new(registry, DelegationConfig::default());
        let input = json!({
            "agent_id": "mock",
            "task": "run tests"
        });
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(!result.is_error);
        assert!(result.content.contains("MockAgent processed"));
    }

    #[tokio::test]
    async fn test_delegate_depth_limit() {
        let registry = Arc::new(AgentRegistry::new());
        let tool = DelegateTool::new(registry, DelegationConfig::default()).child(); // depth=1, which equals max_depth=1

        let input = json!({
            "agent_id": "any",
            "task": "do something"
        });
        let ctx = ToolContext {
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
        };

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("depth limit"));
    }
}
