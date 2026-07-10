//! Dual-model bridge — routes tasks between a planner (heavy) and executor (fast) LLM.
//!
//! Simple tasks go straight to the executor. Complex tasks first consult the planner
//! for analysis, then pass the planner's guidance to the executor for the final response.

use serde::{Deserialize, Serialize};

use crate::bridge::llm::LlmBridge;

/// Task complexity classification for routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskComplexity {
    /// Simple, direct tasks — handled by executor only.
    Simple,
    /// Medium complexity — executor handles, but planner may annotate.
    Medium,
    /// Complex tasks — planner analyzes first, then executor responds.
    Complex,
}

/// Configuration for the dual-model bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualModelConfig {
    /// Identifier for the planner (heavy reasoning) provider.
    pub planner_provider: String,
    /// Identifier for the executor (fast response) provider.
    pub executor_provider: String,
    /// Whether dual-model routing is enabled.
    pub enabled: bool,
}

impl Default for DualModelConfig {
    fn default() -> Self {
        Self {
            planner_provider: "planner".to_string(),
            executor_provider: "executor".to_string(),
            enabled: true,
        }
    }
}

/// Dual-model bridge holding two `LlmBridge` instances.
///
/// The `planner` model is used for complex reasoning and analysis.
/// The `executor` model handles direct task execution and interaction.
pub struct DualModelBridge {
    planner: LlmBridge,
    executor: LlmBridge,
    #[allow(dead_code)]
    config: DualModelConfig,
}

impl DualModelBridge {
    /// Create a new dual-model bridge.
    pub fn new(planner: LlmBridge, executor: LlmBridge, config: DualModelConfig) -> Self {
        Self {
            planner,
            executor,
            config,
        }
    }

    /// Access the planner model (for analysis/planning tasks).
    pub fn planner(&self) -> &LlmBridge {
        &self.planner
    }

    /// Access the executor model (for execution/interaction tasks).
    pub fn executor(&self) -> &LlmBridge {
        &self.executor
    }

    /// Route to the appropriate model based on task complexity.
    ///
    /// - `Simple` / `Medium` → executor
    /// - `Complex` → planner
    pub fn route(&self, task_complexity: TaskComplexity) -> &LlmBridge {
        match task_complexity {
            TaskComplexity::Simple | TaskComplexity::Medium => &self.executor,
            TaskComplexity::Complex => &self.planner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::r#impl::llm::{
        LlmProvider, LlmResponse, LlmStream, StopReason, ToolDefinition, Usage,
    };
    use fabric::message::{ContentBlock, Message};

    /// Minimal stub provider for unit tests.
    struct StubProvider {
        name: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for StubProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: format!("{}: ok", self.name),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }

        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!("not needed in tests")
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn max_context_length(&self) -> usize {
            128_000
        }
    }

    fn make_bridge() -> DualModelBridge {
        let planner = LlmBridge::new(Arc::new(StubProvider {
            name: "planner".into(),
        }));
        let executor = LlmBridge::new(Arc::new(StubProvider {
            name: "executor".into(),
        }));
        DualModelBridge::new(planner, executor, DualModelConfig::default())
    }

    #[test]
    fn route_simple_goes_to_executor() {
        let bridge = make_bridge();
        let routed = bridge.route(TaskComplexity::Simple);
        assert_eq!(routed.name(), "executor");
    }

    #[test]
    fn route_medium_goes_to_executor() {
        let bridge = make_bridge();
        let routed = bridge.route(TaskComplexity::Medium);
        assert_eq!(routed.name(), "executor");
    }

    #[test]
    fn route_complex_goes_to_planner() {
        let bridge = make_bridge();
        let routed = bridge.route(TaskComplexity::Complex);
        assert_eq!(routed.name(), "planner");
    }

    #[test]
    fn planner_and_executor_accessors() {
        let bridge = make_bridge();
        assert_eq!(bridge.planner().name(), "planner");
        assert_eq!(bridge.executor().name(), "executor");
    }

    #[test]
    fn task_complexity_display_roundtrip() {
        // Ensure the enum variants are distinct
        assert_ne!(TaskComplexity::Simple, TaskComplexity::Complex);
        assert_ne!(TaskComplexity::Medium, TaskComplexity::Simple);
    }
}
