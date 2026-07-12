//! LlmPulse — the heart of the system.
//!
//! Periodically broadcasts cognitive energy to EventBus.
//! Agents consume this energy to think and act.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::watch;
use uuid::Uuid;

use fabric::evolution::CognitivePulseEvent;
use fabric::CommunicationBus;
use fabric::EventType;

use super::scheduler::LlmScheduler;

/// Configuration for LlmPulse.
#[derive(Debug, Clone)]
pub struct PulseConfig {
    /// Interval between pulses.
    pub interval: Duration,
    /// Token budget per pulse.
    pub token_budget_per_pulse: u32,
}

impl Default for PulseConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            token_budget_per_pulse: 100_000,
        }
    }
}

/// The heart — periodically broadcasts cognitive energy to EventBus.
pub struct LlmPulse {
    scheduler: Arc<LlmScheduler>,
    bus: Arc<CommunicationBus>,
    config: PulseConfig,
}

impl LlmPulse {
    pub fn new(
        scheduler: Arc<LlmScheduler>,
        bus: Arc<CommunicationBus>,
        config: PulseConfig,
    ) -> Self {
        Self {
            scheduler,
            bus,
            config,
        }
    }

    /// Start the pulse loop. Runs until shutdown signal.
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.config.interval);
        tracing::info!("LlmPulse started (interval: {:?})", self.config.interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.pulse().await {
                        tracing::error!("LlmPulse error: {}", e);
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("LlmPulse shutting down");
                    break;
                }
            }
        }
    }

    /// Emit one cognitive pulse.
    async fn pulse(&self) -> Result<()> {
        let health = self.scheduler.health_check().await;

        let event = CognitivePulseEvent {
            pulse_id: Uuid::new_v4(),
            timestamp: Utc::now().to_rfc3339(),
            available_tokens: self.config.token_budget_per_pulse,
            provider_health: health,
        };

        let json_payload = serde_json::to_value(&event)?;

        self.bus
            .publish_event_v2(&EventType::CognitivePulse, "llm_pulse", json_payload)
            .await
    }

    /// Emit a single pulse (for testing).
    pub async fn pulse_once(&self) -> Result<()> {
        self.pulse().await
    }
}
