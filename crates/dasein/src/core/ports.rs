//! D3 Dasein/Metacog authority convergence (Agent Kernel V2).
//!
//! Defines the Dasein-owned self-mutation authority and the post-settlement
//! consumer seam.  Rich types that today live in `fabric::dasein` return to
//! this owner crate at the cutover; repository/sandbox/coding-evaluator move
//! to adapters; the Runtime durable outbox feeds the two post-settlement
//! consumers.  This seam is contract/port only — no writer change, no facade
//! deletion (that is D3 PR-C / XRET-02).

use async_trait::async_trait;

/// A Dasein-owned self-mutation request.  Only Dasein submits Self mutations;
/// Metacog observes/evaluates/proposes but never mutates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfMutation {
    pub version: u64,
    pub mutation: String,
}

/// The single authority for Self mutation.
#[async_trait]
pub trait SelfMutationAuthority: Send + Sync {
    /// Commit a self mutation.  Only this authority may submit Self mutation.
    async fn commit(&self, mutation: SelfMutation) -> Result<u64, DaseinError>;
}

/// A post-settlement consumer fed by the Runtime durable outbox.
#[async_trait]
pub trait PostSettlementConsumer: Send + Sync {
    async fn on_settlement(&self, session: String, turn: String) -> Result<(), DaseinError>;
}

/// Typed Dasein errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DaseinError {
    #[error("self mutation rejected by authority")]
    MutationRejected,
    #[error("settlement consumer unavailable")]
    ConsumerUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestAuthority {
        version: AtomicU64,
    }

    #[async_trait]
    impl SelfMutationAuthority for TestAuthority {
        async fn commit(&self, mutation: SelfMutation) -> Result<u64, DaseinError> {
            let v = self.version.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = mutation;
            Ok(v)
        }
    }

    #[tokio::test]
    async fn authority_commits_self_mutation() {
        let authority = TestAuthority {
            version: AtomicU64::new(0),
        };
        let v = authority
            .commit(SelfMutation {
                version: 1,
                mutation: "reflect".into(),
            })
            .await
            .unwrap();
        assert_eq!(v, 1);
    }
}
