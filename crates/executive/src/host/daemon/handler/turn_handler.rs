//! Per-turn cancellation token management.
//!
//! Provides token creation and cancellation for graceful shutdown
//! of in-flight chat turns.

use super::RequestHandler;
impl RequestHandler {
    /// Cancel any in-flight chat turn by requesting an interrupt on the
    /// runtime and cancelling the per-turn cancellation token.
    pub async fn cancel_current_turn(&self) -> usize {
        let cancelled = self.ports.turn.cancel_current().await;
        tracing::info!(cancelled, "all current turn cancellations requested");
        cancelled
    }

    pub async fn cancel_current_turn_for_principal(
        &self,
        principal_id: fabric::PrincipalId,
    ) -> usize {
        let cancelled = self
            .ports
            .turn
            .cancel_current_for_principal(principal_id)
            .await;
        tracing::info!(cancelled, "current turn cancellation requested");
        cancelled
    }
}
