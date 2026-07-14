//! Cross-process approval gate. When the daemon's guarded runner needs approval,
//! this gate forwards the request over an mpsc channel to the daemon's request
//! handler, which relays it to the connected CLI/TUI and feeds the user's answer
//! back. Fail-safe: if the channel is gone or the responder is dropped → Deny.

use async_trait::async_trait;
use fabric::Clock;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use super::approval::{ApprovalDecision, ApprovalGate, ApprovalRequest};
use aletheon_kernel::chronos::Timer;

/// A pending approval forwarded to the daemon side. The runner blocks on `respond`.
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub respond: oneshot::Sender<ApprovalDecision>,
}

/// Approval gate that forwards to a daemon-side receiver.
pub struct SocketApprovalGate {
    tx: mpsc::Sender<PendingApproval>,
    clock: Arc<dyn Clock>,
}

impl SocketApprovalGate {
    /// Create the gate and the receiver the daemon handler will drain.
    pub fn new(clock: Arc<dyn Clock>) -> (Self, mpsc::Receiver<PendingApproval>) {
        let (tx, rx) = mpsc::channel(8);
        (Self { tx, clock }, rx)
    }
}

#[async_trait]
impl ApprovalGate for SocketApprovalGate {
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision {
        let (respond, wait) = oneshot::channel();
        let pending = PendingApproval {
            request: req.clone(),
            respond,
        };
        if self.tx.send(pending).await.is_err() {
            return ApprovalDecision::Deny; // daemon side gone → fail-safe
        }
        // Bound the wait so a disconnected client can't hang a turn forever.
        match Timer::timeout(&*self.clock, std::time::Duration::from_secs(120), wait).await {
            Ok(Ok(decision)) => decision,
            _ => ApprovalDecision::Deny, // timeout or dropped responder → fail-safe
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    #[tokio::test]
    async fn gate_forwards_request_and_returns_responder_decision() {
        let clock = Arc::new(TestClock::default());
        let (gate, mut rx) = SocketApprovalGate::new(clock);

        // Spawn a task that receives the pending approval and responds Approve.
        let handle = tokio::spawn(async move {
            let pending = rx.recv().await.expect("channel should not be closed");
            assert_eq!(pending.request.tool, "bash_exec");
            pending
                .respond
                .send(ApprovalDecision::Approve)
                .expect("receiver should be alive");
        });

        let req = ApprovalRequest {
            tool: "bash_exec".into(),
            action_summary: "rm -rf /tmp/x".into(),
            risk_level: "high".into(),
            detail: None,
        };
        let decision = gate.request(&req).await;
        assert_eq!(decision, ApprovalDecision::Approve);
        handle.await.expect("responder task panicked");
    }

    #[tokio::test]
    async fn gate_denies_if_channel_dropped() {
        let clock = Arc::new(TestClock::default());
        let (gate, rx) = SocketApprovalGate::new(clock);
        // Drop the receiver immediately — simulates daemon side gone.
        drop(rx);

        let req = ApprovalRequest {
            tool: "bash_exec".into(),
            action_summary: "ls".into(),
            risk_level: "low".into(),
            detail: None,
        };
        let decision = gate.request(&req).await;
        assert_eq!(decision, ApprovalDecision::Deny);
    }
}
