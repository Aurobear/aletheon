use std::sync::Arc;

use application::evaluation::EvaluationAcceptancePort;
use async_trait::async_trait;

/// Agora-backed projection of an authoritative deterministic evaluation.
pub struct AgoraEvaluationAcceptance {
    controller: agora::host_acceptance::HostAcceptanceController,
}

impl AgoraEvaluationAcceptance {
    pub fn new(agora: Arc<dyn agora::AgoraService>) -> Self {
        Self {
            controller: agora::host_acceptance::HostAcceptanceController::new(agora),
        }
    }
}

#[async_trait]
impl EvaluationAcceptancePort for AgoraEvaluationAcceptance {
    async fn settle(
        &self,
        session_id: &str,
        receipt: &contracts::EvaluationReceiptRef,
        owner: contracts::ProcessId,
    ) -> anyhow::Result<()> {
        self.controller
            .settle_evaluation(session_id, receipt, owner)
            .await?;
        Ok(())
    }
}
