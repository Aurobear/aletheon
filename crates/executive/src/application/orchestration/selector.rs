use anyhow::Result;
use tracing::{debug, info};

use super::agent::AgentResponse;
use super::registry::AgentRegistry;
use fabric::message::{ContentBlock, Message};
use fabric::LlmProvider;

/// Selector configuration.
pub struct SelectorConfig {
    pub max_selection_attempts: usize,
    pub fallback_agent_id: Option<String>,
}

impl Default for SelectorConfig {
    fn default() -> Self {
        Self {
            max_selection_attempts: 3,
            fallback_agent_id: None,
        }
    }
}

/// Selector strategy -- LLM selects which agent to route a message to.
pub struct SelectorStrategy {
    registry: AgentRegistry,
    config: SelectorConfig,
}

impl SelectorStrategy {
    pub fn new(registry: AgentRegistry, config: SelectorConfig) -> Self {
        Self { registry, config }
    }

    /// Select the best agent for a given message using LLM routing.
    pub async fn select_agent(&self, message: &str, llm: &dyn LlmProvider) -> Result<String> {
        let agent_ids = self.registry.list_ids().await;
        if agent_ids.is_empty() {
            return Err(anyhow::anyhow!("No agents registered"));
        }

        if agent_ids.len() == 1 {
            return Ok(agent_ids[0].clone());
        }

        // Build agent descriptions for LLM
        let mut agent_descriptions = Vec::new();
        for id in &agent_ids {
            if let Some(agent) = self.registry.get(id).await {
                let agent_read = agent.read().await;
                let caps: Vec<&str> = agent_read
                    .capabilities()
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect();
                agent_descriptions.push(format!(
                    "- {}: {} (capabilities: {})",
                    id,
                    agent_read.description(),
                    caps.join(", ")
                ));
            }
        }

        // Ask LLM to select
        let selection_prompt = format!(
            "You are a routing agent. Given the user message below, select the most appropriate agent from the list.\n\nAvailable agents:\n{}\n\nUser message: {}\n\nRespond with ONLY the agent ID (e.g., 'fs_agent'). No explanation.",
            agent_descriptions.join("\n"),
            message
        );

        let messages = vec![Message::user(&selection_prompt)];
        let tools = vec![];
        let response = llm.complete(&messages, &tools).await?;

        let selected = response
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.trim().to_string()),
                _ => None,
            })
            .unwrap_or_default();

        debug!(raw_selection = %selected, "LLM raw selection");

        // Validate selection
        if agent_ids.contains(&selected) {
            info!(agent_id = %selected, "Selector chose agent");
            Ok(selected)
        } else {
            // Try fuzzy match
            let fuzzy = agent_ids.iter().find(|id| selected.contains(id.as_str()));
            if let Some(id) = fuzzy {
                info!(agent_id = %id, "Selector fuzzy-matched agent");
                Ok(id.clone())
            } else if let Some(fallback) = &self.config.fallback_agent_id {
                info!(agent_id = %fallback, "Selector using fallback agent");
                Ok(fallback.clone())
            } else {
                Err(anyhow::anyhow!("LLM selected unknown agent: '{selected}'"))
            }
        }
    }

    /// Route a message to the selected agent and get response.
    pub async fn route(&self, message: &str, llm: &dyn LlmProvider) -> Result<AgentResponse> {
        let agent_id = self.select_agent(message, llm).await?;

        let agent = self
            .registry
            .get(&agent_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Agent '{agent_id}' disappeared"))?;

        let agent_read = agent.read().await;
        let msg = Message::user(message);
        agent_read.on_message(msg).await
    }
}
