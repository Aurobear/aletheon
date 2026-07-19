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
    evicted_memory: Option<std::sync::Arc<tokio::sync::Mutex<mnemosyne::RecallMemory>>>,
    verifier: Option<std::sync::Arc<dyn fabric::policy::verifier::Verifier>>,
}

impl LinearCognitiveSessionFactory {
    pub fn new(config: HarnessConfig, clock: std::sync::Arc<dyn fabric::Clock>) -> Self {
        Self {
            config,
            clock,
            evicted_memory: None,
            verifier: None,
        }
    }

    pub fn with_verifier(mut self, verifier: std::sync::Arc<dyn fabric::policy::verifier::Verifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    pub fn with_evicted_memory(
        mut self,
        memory: std::sync::Arc<tokio::sync::Mutex<mnemosyne::RecallMemory>>,
    ) -> Self {
        self.evicted_memory = Some(memory);
        self
    }
}

#[async_trait]
impl CognitiveSessionFactory for LinearCognitiveSessionFactory {
    async fn create(
        &self,
        session: &SessionRecord,
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
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
            },
        )))
    }

    async fn create_configured(
        &self,
        session: &SessionRecord,
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
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
            },
        )))
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
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
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
            },
        )))
    }
}

fn evicted_callback(
    memory: Option<std::sync::Arc<tokio::sync::Mutex<mnemosyne::RecallMemory>>>,
    session: &SessionRecord,
) -> Option<std::sync::Arc<dyn Fn(Vec<fabric::Message>) + Send + Sync>> {
    let memory = memory?;
    let session_id = session.id.0.clone();
    Some(std::sync::Arc::new(move |messages| {
        let memory = memory.clone();
        let session_id = session_id.clone();
        tokio::spawn(async move {
            let metadata = serde_json::json!({
                "scope_key": format!("session:{session_id}"),
                "sensitivity": mnemosyne::MemorySensitivity::Internal,
                "sensitivity_ord": 1,
                "authority": mnemosyne::MemoryAuthority::RawExperience,
                "provenance": "compaction_evicted"
            })
            .to_string();
            let memory = memory.lock().await;
            for message in messages {
                let content = serde_json::to_string(&message)
                    .unwrap_or_else(|_| "[unserializable evicted message]".to_string());
                if let Err(error) =
                    memory.store(&session_id, "compaction_evicted", &content, Some(&metadata))
                {
                    tracing::warn!(%error, %session_id, "failed to capture evicted compaction message");
                }
            }
        });
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evicted_callback_captures_session_scoped_observed_memory() {
        let dir = tempfile::tempdir().unwrap();
        let memory = std::sync::Arc::new(tokio::sync::Mutex::new(
            mnemosyne::RecallMemory::new(
                &dir.path().join("recall.db"),
                std::sync::Arc::new(aletheon_kernel::chronos::SystemClock::new()),
            )
            .unwrap(),
        ));
        let session = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("session-c1".into()),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        };
        let callback = evicted_callback(Some(memory.clone()), &session).unwrap();
        callback(vec![fabric::Message::user("durable compaction marker")]);

        let mut found = Vec::new();
        for _ in 0..20 {
            tokio::task::yield_now().await;
            found = memory
                .lock()
                .await
                .search_in_session("session-c1", "durable compaction marker", 5)
                .unwrap();
            if !found.is_empty() {
                break;
            }
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].entry_type, "compaction_evicted");
        let metadata: serde_json::Value =
            serde_json::from_str(found[0].metadata.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["scope_key"], "session:session-c1");
        assert_eq!(metadata["authority"], "raw_experience");
        assert_eq!(metadata["sensitivity"], "internal");
    }

    #[test]
    fn missing_memory_store_is_explicit_noop_fallback() {
        let session = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("session-noop".into()),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        };
        assert!(evicted_callback(None, &session).is_none());
    }
}
