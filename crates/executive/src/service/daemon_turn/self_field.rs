//! SelfField interaction methods on `DaemonTurnOrchestrator` and `TurnPipeline`.

use super::orchestrator::DaemonTurnOrchestrator;
use crate::service::turn_pipeline::TurnPipeline;
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

    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            let _mood = dasein.quick_mood_update(turn_text);
            tracing::info!(turn = turn, "Dasein mood updated via coordinator");
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

    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            let _mood = dasein.quick_mood_update(turn_text);
            tracing::info!(turn = turn, "Dasein mood updated via coordinator");
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
