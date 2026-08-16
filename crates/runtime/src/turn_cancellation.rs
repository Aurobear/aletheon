//! Runtime-owned Turn cancellation controller (RA-04).
//!
//! Cancellation is a typed lifecycle signal, not just a token.  The first
//! reason is retained so a deadline-bearing Turn cancelled by a user is not
//! misclassified during terminal settlement or disconnect recovery.

use ::contracts::CancelReason;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct TurnCancellation {
    token: CancellationToken,
    reason: Arc<Mutex<Option<CancelReason>>>,
}

impl Default for TurnCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnCancellation {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Arc::new(Mutex::new(None)),
        }
    }

    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn request(&self, reason: CancelReason) {
        let mut recorded = self
            .reason
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if recorded.is_none() {
            *recorded = Some(reason);
        }
        drop(recorded);
        self.token.cancel();
    }

    pub fn reason(&self) -> Option<CancelReason> {
        self.reason
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub async fn cancelled(&self) {
        self.token.cancelled().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_reason_wins_and_token_is_cancelled() {
        let cancellation = TurnCancellation::new();
        cancellation.request(CancelReason::User);
        cancellation.request(CancelReason::DeadlineExceeded);
        assert_eq!(cancellation.reason(), Some(CancelReason::User));
        assert!(cancellation.token().is_cancelled());
        cancellation.cancelled().await;
    }
}
