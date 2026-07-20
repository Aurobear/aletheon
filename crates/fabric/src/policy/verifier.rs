//! Result-verification seam (M-C). A `Verifier` inspects a candidate final
//! answer and either accepts it or rejects it with a reason so the runtime can
//! request a revision. The default `NoopVerifier` always accepts, preserving
//! behavior when no verifier is configured.

use crate::message::Message;
use async_trait::async_trait;

/// Outcome of verifying a candidate final answer.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// The answer is acceptable; return it to the caller.
    Accept,
    /// The answer is rejected; `reason` is fed back to the model for a revision.
    Reject { reason: String },
}

/// Inspects a candidate final answer in the context of the conversation.
#[async_trait]
pub trait Verifier: Send + Sync {
    /// Verify the model's final text. `messages` is the full conversation so far
    /// (system + user + assistant + tool turns), for context-aware checks.
    async fn verify(&self, final_text: &str, messages: &[Message]) -> Verdict;
}

/// The default verifier: accepts everything (no behavior change).
pub struct NoopVerifier;

#[async_trait]
impl Verifier for NoopVerifier {
    async fn verify(&self, _final_text: &str, _messages: &[Message]) -> Verdict {
        Verdict::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[tokio::test]
    async fn noop_verifier_always_accepts() {
        let v = NoopVerifier;
        let msgs = vec![Message::user("hi")];
        assert!(matches!(
            v.verify("any answer", &msgs).await,
            Verdict::Accept
        ));
    }

    #[tokio::test]
    async fn reject_carries_reason() {
        struct Always;
        #[async_trait::async_trait]
        impl Verifier for Always {
            async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
                Verdict::Reject {
                    reason: "nope".into(),
                }
            }
        }
        match Always.verify("x", &[]).await {
            Verdict::Reject { reason } => assert_eq!(reason, "nope"),
            _ => panic!("expected reject"),
        }
    }
}
