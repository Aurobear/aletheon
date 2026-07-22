//! Per-turn cancellation token management.
//!
//! Provides token creation and cancellation for graceful shutdown
//! of in-flight chat turns.

use super::RequestHandler;
impl RequestHandler {
    /// Cancel any in-flight chat turn by requesting an interrupt on the
    /// runtime and cancelling the per-turn cancellation token.
    pub async fn cancel_current_turn(&self) {
        self.ports.turn.cancel_current().await;
    }
}
