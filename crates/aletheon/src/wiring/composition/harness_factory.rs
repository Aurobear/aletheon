//! General production cognitive-session factory composition.
//!
//! The concrete factory core (`HarnessCognitiveSessionFactory`) is owned by
//! `cognit::harness::factory`; this module is the binary-owned adapter that
//! turns the effective `CognitiveRuntimeConfig` into a `HarnessConfig` and constructs
//! the production General factory. It replaces the retired
//! `crate::wiring::application::harness_factory` production entry point.

use std::sync::Arc;

use crate::config::CognitiveRuntimeConfig;
use contracts::Clock;
use cognit::harness::{
    CognitiveSessionFactory, HarnessCognitiveSessionFactory, HarnessConfig,
    selected_harness_kind,
};

/// Assemble the General production cognitive-session factory from effective
/// daemon configuration. `memory` and `dasein` are accepted for signature
/// compatibility with the daemon bootstrap; the Harness core wires its own
/// session-log persistence and policy hooks.
pub fn production_cognitive_session_factory(
    config: &CognitiveRuntimeConfig,
    clock: Arc<dyn Clock>,
    _memory: Arc<tokio::sync::Mutex<mnemosyne::runtime::RecallMemory>>,
    _dasein: Arc<dyn dasein::DaseinOps>,
) -> Arc<dyn CognitiveSessionFactory> {
    tracing::info!(
        harness = "linear",
        configured_harness = selected_harness_kind(config.harness_kind),
        "linear cognitive session factory composed"
    );
    Arc::new(HarnessCognitiveSessionFactory::new(
        harness_config_from_runtime(config),
        clock,
    ))
}

/// Map the effective cognitive runtime configuration onto the Harness configuration
/// consumed by the cognitive session factory.
pub fn harness_config_from_runtime(config: &CognitiveRuntimeConfig) -> HarnessConfig {
    HarnessConfig {
        max_iterations: config.max_iterations,
        compaction_enabled: config.compaction_enabled,
        compaction_threshold_percent: config.compaction_threshold_percent,
        compaction_v2: config.compaction_v2,
        tail_token_budget: config.tail_token_budget,
        target_summary_chars: config.target_summary_chars,
        context_window_tokens: config.context_window_tokens,
        max_tool_calls: config.agent_loop.max_tool_calls,
        reflection_interval: config.agent_loop.reflection_interval,
        reflection_tool_call_limit: config.agent_loop.reflection_tool_call_limit,
        circuit_breaker_max_repeats: config.circuit_breaker.max_repeats,
        circuit_breaker_window_size: config.circuit_breaker.window_size,
        learning_enabled: config.learning_enabled,
    }
}
