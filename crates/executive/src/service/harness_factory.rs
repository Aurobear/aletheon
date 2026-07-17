use async_trait::async_trait;
use cognit::harness::config::HarnessConfig;
use fabric::SessionRecord;
use mnemosyne::AdvancedCompressor;
use tokio_util::sync::CancellationToken;

use crate::core::config::ExecutiveConfig;
use crate::service::turn_policy::TurnPolicy;

#[async_trait]
pub trait CognitiveSessionFactory: Send + Sync {
    async fn create(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>>;

    async fn create_configured(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        _config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.create(session, policy, cancellation).await
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        _batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.create_configured(session, policy, config, cancellation)
            .await
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
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            self.config.clone(),
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor(&self.config)),
                batch_planner: None,
            },
        )))
    }

    async fn create_configured(
        &self,
        _session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let compactor = compactor(&config);
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            config,
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor),
                batch_planner: None,
            },
        )))
    }

    async fn create_configured_with_batch_planner(
        &self,
        _session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let compactor = compactor(&config);
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            config,
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor),
                batch_planner,
            },
        )))
    }
}

fn compactor(config: &HarnessConfig) -> Box<dyn fabric::CompactorTrait> {
    let effective_tail = if config.tail_token_budget * 4 < config.context_window_tokens {
        config.context_window_tokens / 8
    } else {
        config.tail_token_budget
    };
    Box::new(AdvancedCompressor::new(
        effective_tail,
        config.target_summary_chars,
        config.context_window_tokens,
    ))
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
