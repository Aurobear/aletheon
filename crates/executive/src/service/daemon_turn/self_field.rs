//! SelfField interaction methods on `DaemonTurnOrchestrator` and `TurnPipeline`.

use super::orchestrator::DaemonTurnOrchestrator;
use crate::service::turn_pipeline::TurnPipeline;
use fabric::dasein::OutcomeStatus;
use fabric::{Context as AbiContext, Intent, SelfFieldOps, Verdict};

#[allow(dead_code)]
impl DaemonTurnOrchestrator {
    pub(crate) async fn sf_review(
        &self,
        intent: &Intent,
        ctx: &AbiContext,
    ) -> anyhow::Result<Verdict> {
        let sf = self.subsystems.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
        let sf = self.subsystems.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str, status: OutcomeStatus) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            match dasein
                .record_outcome(turn_text, status, "daemon-turn-orchestrator")
                .await
            {
                Ok(receipt) => tracing::info!(
                    turn,
                    version = receipt.current_version.0,
                    "Dasein outcome transition accepted"
                ),
                Err(error) => tracing::warn!(turn, %error, "Dasein outcome transition rejected"),
            }
        }
    }

    pub(crate) async fn compose_memory_block(&self) -> String {
        let mut queue = self.subsystems.session.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);
        let items: Vec<String> = updates.iter().map(|m| format!("- {}", m)).collect();
        format!("<memory-update>\n{}\n</memory-update>", items.join("\n"))
    }
}

impl TurnPipeline {
    pub(crate) async fn sf_review(
        &self,
        intent: &Intent,
        ctx: &AbiContext,
    ) -> anyhow::Result<Verdict> {
        let sf = self.subsystems.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
        let sf = self.subsystems.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str, status: OutcomeStatus) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            match dasein
                .record_outcome(turn_text, status, "turn-pipeline")
                .await
            {
                Ok(receipt) => tracing::info!(
                    turn,
                    version = receipt.current_version.0,
                    "Dasein outcome transition accepted"
                ),
                Err(error) => tracing::warn!(turn, %error, "Dasein outcome transition rejected"),
            }
        }
    }

    pub(crate) async fn compose_memory_block(&self) -> String {
        let mut queue = self.subsystems.session.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);
        let items: Vec<String> = updates.iter().map(|m| format!("- {}", m)).collect();
        format!("<memory-update>\n{}\n</memory-update>", items.join("\n"))
    }
}
