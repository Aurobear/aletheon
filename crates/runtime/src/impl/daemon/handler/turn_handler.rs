//! Per-turn cancellation token management.
//!
//! Provides token creation and cancellation for graceful shutdown
//! of in-flight chat turns.

use super::RequestHandler;
use base::ui_event::InterruptReason;
use tokio_util::sync::CancellationToken;

impl RequestHandler {
    /// Cancel any in-flight chat turn by requesting an interrupt on the
    /// runtime and cancelling the per-turn cancellation token.
    pub async fn cancel_current_turn(&self) {
        // Set the interrupt flag on the runtime so the ReAct loop can
        // detect and handle the cancellation during tool execution.
        {
            let state = self.state.lock().await;
            let flag = state.runtime.interrupt_flag();
            flag.request(InterruptReason::Timeout);
        }

        // Cancel the per-turn token (created fresh by each handle_chat call).
        let mut token = self.cancel_token.lock().await;
        if let Some(ct) = token.take() {
            ct.cancel();
        }
    }

    /// Create a fresh CancellationToken for the current chat turn.
    /// The returned token is stored so it can be cancelled during shutdown.
    pub async fn begin_turn_token(&self) -> CancellationToken {
        let ct = CancellationToken::new();
        let mut token = self.cancel_token.lock().await;
        *token = Some(ct.clone());
        ct
    }
}
