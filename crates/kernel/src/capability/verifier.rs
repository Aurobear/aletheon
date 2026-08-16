//! K3 authorization evidence verifier (Agent Kernel V2).
//!
//! Defines an opaque single-use authorization proof and the verifier port.
//! The Application resolution adapter implements the verifier (it never
//! exposes an ApprovalStore/repository to the Kernel).  `DecisionRequestId`
//! is never constructed by a permit issuer here — the static/contract gate
//! forbids it.  When the verifier is unavailable, execution fails closed;
//! consumed evidence is never reused.

use serde::{Deserialize, Serialize};

/// Opaque single-use authorization proof.  Constructed only by the verifier
/// after validating evidence; never by a permit issuer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueProof {
    /// Binding nonce — single-use.
    pub nonce: u64,
    /// Expiry (monotonic ms).  Expired proof fails closed.
    pub expires_at_ms: u64,
    /// Scope the proof authorizes.
    pub scope: String,
    /// Generation of the authorizing decision.
    pub generation: u64,
}

/// The verifier port the Kernel calls.  Implemented by the Application
/// resolution adapter (which validates digest/principal/scope/revision/
/// expiry/nonce/single-use).  The Kernel never sees the ApprovalStore.
#[async_trait::async_trait]
pub trait AuthorizationEvidenceVerifier: Send + Sync {
    /// Verify + consume a single-use proof.  Returns the authorized scope on
    /// success.  Fail-closed: forged/expired/wrong-scope/double-consumed and
    /// verifier-unavailable all return Err.
    async fn verify_and_consume(&self, proof: OpaqueProof) -> Result<String, EvidenceError>;
}

/// Typed evidence failures.  All fail closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("verifier unavailable")]
    VerifierUnavailable,
    #[error("proof already consumed (single-use violated)")]
    AlreadyConsumed,
    #[error("proof expired")]
    Expired,
    #[error("proof scope mismatch")]
    WrongScope,
    #[error("proof generation mismatch")]
    WrongGeneration,
    #[error("forged or malformed proof")]
    Forged,
}

/// In-memory single-use verifier for testing.  Production implementation is
/// the Application resolution adapter.
#[derive(Default)]
pub struct InMemoryEvidenceVerifier {
    consumed: std::sync::Mutex<std::collections::HashSet<u64>>,
    now_ms: std::sync::atomic::AtomicU64,
}

impl InMemoryEvidenceVerifier {
    pub fn new(now_ms: u64) -> Self {
        Self {
            consumed: std::sync::Mutex::new(std::collections::HashSet::new()),
            now_ms: std::sync::atomic::AtomicU64::new(now_ms),
        }
    }
}

#[async_trait::async_trait]
impl AuthorizationEvidenceVerifier for InMemoryEvidenceVerifier {
    async fn verify_and_consume(&self, proof: OpaqueProof) -> Result<String, EvidenceError> {
        let now = self.now_ms.load(std::sync::atomic::Ordering::Relaxed);
        if now >= proof.expires_at_ms {
            return Err(EvidenceError::Expired);
        }
        let mut consumed = self.consumed.lock().unwrap();
        if consumed.contains(&proof.nonce) {
            return Err(EvidenceError::AlreadyConsumed);
        }
        if proof.scope.trim().is_empty() {
            return Err(EvidenceError::Forged);
        }
        consumed.insert(proof.nonce);
        Ok(proof.scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(nonce: u64, expires: u64) -> OpaqueProof {
        OpaqueProof {
            nonce,
            expires_at_ms: expires,
            scope: "tool.x".into(),
            generation: 1,
        }
    }

    #[tokio::test]
    async fn single_use_fails_closed_on_second_use() {
        let verifier = InMemoryEvidenceVerifier::new(1_000);
        let scope = verifier.verify_and_consume(proof(7, 2_000)).await.unwrap();
        assert_eq!(scope, "tool.x");
        assert_eq!(
            verifier.verify_and_consume(proof(7, 2_000)).await,
            Err(EvidenceError::AlreadyConsumed)
        );
    }

    #[tokio::test]
    async fn expired_fails_closed() {
        let verifier = InMemoryEvidenceVerifier::new(3_000);
        assert_eq!(
            verifier.verify_and_consume(proof(9, 2_000)).await,
            Err(EvidenceError::Expired)
        );
    }
}
