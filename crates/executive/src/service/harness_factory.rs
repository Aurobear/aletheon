use async_trait::async_trait;
use cognit::harness::build_harness;
use cognit::harness::config::HarnessConfig;
use cognit::harness::linear::ReActLoop;
use fabric::SessionRecord;
use mnemosyne::AdvancedCompressor;

use crate::core::config::ExecutiveConfig;
use crate::service::turn_policy::TurnPolicy;

#[async_trait]
pub trait CognitiveSessionFactory: Send + Sync {
    async fn create(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>>;

    async fn create_configured(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        _config: HarnessConfig,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.create(session, policy).await
    }
}

pub struct LinearCognitiveSessionFactory {
    config: HarnessConfig,
    clock: std::sync::Arc<dyn fabric::Clock>,
}

impl LinearCognitiveSessionFactory {
    pub fn new(config: HarnessConfig, clock: std::sync::Arc<dyn fabric::Clock>) -> Self {
        Self { config, clock }
    }
}

#[async_trait]
impl CognitiveSessionFactory for LinearCognitiveSessionFactory {
    async fn create(
        &self,
        _session: &SessionRecord,
        _policy: &TurnPolicy,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            self.config.clone(),
            self.clock.clone(),
        )))
    }

    async fn create_configured(
        &self,
        _session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            config,
            self.clock.clone(),
        )))
    }
}

pub fn harness_config_from_executive(config: &ExecutiveConfig) -> HarnessConfig {
    HarnessConfig {
        max_iterations: config.max_iterations,
        compaction_enabled: config.compaction_enabled,
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

pub fn build_configured_react_loop(
    config: &ExecutiveConfig,
    clock: std::sync::Arc<dyn fabric::Clock>,
) -> ReActLoop {
    build_react_loop(config, harness_config_from_executive(config), clock)
}

pub fn build_react_loop(
    config: &ExecutiveConfig,
    harness_config: HarnessConfig,
    clock: std::sync::Arc<dyn fabric::Clock>,
) -> ReActLoop {
    let effective_tail = if config.tail_token_budget * 4 < config.context_window_tokens {
        config.context_window_tokens / 8
    } else {
        config.tail_token_budget
    };
    let compressor = Box::new(AdvancedCompressor::new(
        effective_tail,
        config.target_summary_chars,
        config.context_window_tokens,
    )) as Box<dyn cognit::harness::linear::CompactorTrait>;
    build_harness(config.harness_kind, harness_config, compressor, clock)
}
