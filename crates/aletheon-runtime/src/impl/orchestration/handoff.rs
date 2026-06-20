use anyhow::Result;
use tracing::{info, warn};

use super::agent::{AgentResponse, AgentResponseStatus};
use super::budget::IterationBudget;
use super::registry::AgentRegistry;
use aletheon_abi::message::Message;

/// Handoff configuration.
pub struct HandoffConfig {
    pub max_handoffs: usize,
    pub max_iterations_per_agent: usize,
}

impl Default for HandoffConfig {
    fn default() -> Self {
        Self {
            max_handoffs: 5,
            max_iterations_per_agent: 50,
        }
    }
}

/// Handoff strategy -- agents can explicitly hand off to other agents.
///
/// Flow:
/// 1. Initial agent receives message
/// 2. Agent processes and may return Handoff(target_agent, reason)
/// 3. Message is forwarded to target agent
/// 4. Repeat until agent returns Completed or max handoffs reached
pub struct HandoffStrategy {
    registry: AgentRegistry,
    config: HandoffConfig,
}

/// Extended response that includes handoff requests.
#[derive(Debug, Clone)]
pub enum HandoffResponse {
    /// Agent completed normally.
    Completed(AgentResponse),
    /// Agent requests handoff to another agent.
    Handoff {
        target_agent: String,
        reason: String,
        context: String,
    },
}

impl HandoffStrategy {
    pub fn new(registry: AgentRegistry, config: HandoffConfig) -> Self {
        Self { registry, config }
    }

    /// Execute a task with handoff support.
    pub async fn execute(&self, initial_agent_id: &str, message: &str) -> Result<AgentResponse> {
        let _budget =
            IterationBudget::new(self.config.max_iterations_per_agent * self.config.max_handoffs);
        let mut current_agent_id = initial_agent_id.to_string();
        let mut current_message = message.to_string();
        let mut handoff_count = 0;
        let mut total_iterations = 0;

        loop {
            if handoff_count >= self.config.max_handoffs {
                warn!(count = handoff_count, "Max handoffs reached");
                return Ok(AgentResponse {
                    content: format!(
                        "Max handoffs ({}) reached. Last agent: {}",
                        self.config.max_handoffs, current_agent_id
                    ),
                    tool_calls_made: 0,
                    iterations_used: total_iterations,
                    status: AgentResponseStatus::MaxIterationsReached,
                });
            }

            let agent = self
                .registry
                .get(&current_agent_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", current_agent_id))?;

            info!(agent_id = %current_agent_id, handoff = handoff_count, "Executing with agent");

            let agent_write = agent.write().await;
            let msg = Message::user(&current_message);
            let response = agent_write.on_message(msg).await?;

            total_iterations += response.iterations_used;

            // Check if agent wants to hand off
            // In a real implementation, the agent would signal handoff via a special tool call
            // For now, we check if the response contains a handoff pattern
            if let Some(handoff) = parse_handoff_request(&response.content) {
                info!(
                    from = %current_agent_id,
                    to = %handoff.target_agent,
                    reason = %handoff.reason,
                    "Agent requested handoff"
                );
                current_agent_id = handoff.target_agent;
                current_message = handoff.context;
                handoff_count += 1;
                continue;
            }

            // Agent completed normally
            return Ok(AgentResponse {
                content: response.content,
                tool_calls_made: response.tool_calls_made,
                iterations_used: total_iterations,
                status: response.status,
            });
        }
    }
}

/// Parse handoff request from agent response.
/// Looks for patterns like: [HANDOFF: agent_id] reason
fn parse_handoff_request(content: &str) -> Option<HandoffRequest> {
    // Simple pattern matching — in production, use a structured tool call
    if let Some(start) = content.find("[HANDOFF:") {
        if let Some(end) = content[start..].find(']') {
            let target = content[start + 9..start + end].trim().to_string();
            let reason = content[start + end + 1..]
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            return Some(HandoffRequest {
                target_agent: target,
                reason,
                context: String::new(),
            });
        }
    }
    None
}

struct HandoffRequest {
    target_agent: String,
    reason: String,
    context: String,
}
