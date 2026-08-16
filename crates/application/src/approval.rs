//! APX-02 Approval aggregate/store port (Agent Kernel V2).
//!
//! Owner-seam slice: extracts the Approval aggregate, use cases and the
//! `ApprovalStore` port from the legacy Executive `approval_service.rs` /
//! `approval/repository.rs`.  This seam defines the port and the validation
//! semantics; the legacy SQLite repository remains the single writer until the
//! APX-02 writer cutover.  `DecisionRequestId` source, resolution evidence,
//! expiry, nonce, and single-use state are enforced here — forged / expired /
//! wrong-principal / wrong-scope / double-consumed evidence all fail closed.

use serde::{Deserialize, Serialize};

/// Stable approval challenge identity.  Assigned by the Application owner;
/// never minted by a client.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionRequestId(pub String);

/// The principal an approval was issued to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovingPrincipal(pub String);

/// Scope the approval covers (capability / workspace).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalScope(pub String);

/// Single-use opaque approval grant.  Constructed only by the Application
/// after validating evidence; never by a client.  Not a `contracts` type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueApprovalGrant {
    pub challenge: DecisionRequestId,
    pub principal: ApprovingPrincipal,
    pub scope: ApprovalScope,
    pub revision: u64,
    pub nonce: u64,
}

/// An approval record with single-use/expiry state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: DecisionRequestId,
    pub principal: ApprovingPrincipal,
    pub scope: ApprovalScope,
    pub revision: u64,
    pub expiry_ms: u64,
    pub nonce: u64,
    pub consumed: bool,
}

/// The Approval store port.  The legacy SQLite repository implements this;
/// the Application consumes the port.  Kernel/Dasein never read the store
/// directly — they call their own verifier port (APX-02).
pub trait ApprovalStore: Send + Sync {
    fn load(&self, id: &DecisionRequestId) -> Option<ApprovalRecord>;
    fn mark_consumed(&self, id: &DecisionRequestId) -> Result<(), ApprovalError>;
}

/// Typed approval errors — forged/expired/wrong-principal/wrong-scope/
/// double-consumed all fail closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("approval not found")]
    NotFound,
    #[error("approval already consumed (single-use violated)")]
    AlreadyConsumed,
    #[error("approval expired")]
    Expired,
    #[error("approval is for a different principal")]
    WrongPrincipal,
    #[error("approval scope mismatch")]
    WrongScope,
    #[error("revision generation mismatch")]
    RevisionMismatch,
    #[error("nonce mismatch")]
    NonceMismatch,
}

/// Validates a resolution decision against the store.  Enforces digest,
/// principal, scope, revision/generation, expiry, nonce and single-use — all
/// fail closed.  Returns an opaque grant only when every check passes.
pub fn resolve_decision(
    store: &dyn ApprovalStore,
    id: &DecisionRequestId,
    claimed_principal: &ApprovingPrincipal,
    claimed_scope: &ApprovalScope,
    claimed_revision: u64,
    claimed_nonce: u64,
    now_ms: u64,
) -> Result<OpaqueApprovalGrant, ApprovalError> {
    let record = store.load(id).ok_or(ApprovalError::NotFound)?;

    if record.consumed {
        return Err(ApprovalError::AlreadyConsumed);
    }
    if now_ms >= record.expiry_ms {
        return Err(ApprovalError::Expired);
    }
    if &record.principal != claimed_principal {
        return Err(ApprovalError::WrongPrincipal);
    }
    if &record.scope != claimed_scope {
        return Err(ApprovalError::WrongScope);
    }
    if record.revision != claimed_revision {
        return Err(ApprovalError::RevisionMismatch);
    }
    if record.nonce != claimed_nonce {
        return Err(ApprovalError::NonceMismatch);
    }

    // Single-use: consume atomically; a double resolve fails closed.
    store.mark_consumed(id)?;

    Ok(OpaqueApprovalGrant {
        challenge: record.id,
        principal: record.principal,
        scope: record.scope,
        revision: record.revision,
        nonce: record.nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore(Mutex<HashMap<DecisionRequestId, ApprovalRecord>>);

    impl ApprovalStore for MemStore {
        fn load(&self, id: &DecisionRequestId) -> Option<ApprovalRecord> {
            self.0.lock().unwrap().get(id).cloned()
        }
        fn mark_consumed(&self, id: &DecisionRequestId) -> Result<(), ApprovalError> {
            let mut map = self.0.lock().unwrap();
            let rec = map.get_mut(id).ok_or(ApprovalError::NotFound)?;
            if rec.consumed {
                return Err(ApprovalError::AlreadyConsumed);
            }
            rec.consumed = true;
            Ok(())
        }
    }

    fn record() -> (DecisionRequestId, ApprovalRecord) {
        let id = DecisionRequestId("challenge-1".into());
        let rec = ApprovalRecord {
            id: id.clone(),
            principal: ApprovingPrincipal("alice".into()),
            scope: ApprovalScope("tool.x".into()),
            revision: 3,
            expiry_ms: 1_000_000,
            nonce: 7,
            consumed: false,
        };
        (id, rec)
    }

    #[test]
    fn valid_evidence_resolves_and_is_single_use() {
        let (id, rec) = record();
        let store = MemStore::default();
        store.0.lock().unwrap().insert(id.clone(), rec);

        let grant = resolve_decision(
            &store,
            &id,
            &ApprovingPrincipal("alice".into()),
            &ApprovalScope("tool.x".into()),
            3,
            7,
            500,
        )
        .unwrap();
        assert_eq!(grant.challenge.0, "challenge-1");

        // Second resolve of the same challenge → AlreadyConsumed (fail closed).
        assert_eq!(
            resolve_decision(
                &store,
                &id,
                &ApprovingPrincipal("alice".into()),
                &ApprovalScope("tool.x".into()),
                3,
                7,
                500,
            ),
            Err(ApprovalError::AlreadyConsumed)
        );
    }

    #[test]
    fn wrong_principal_and_expired_fail_closed() {
        let (id, rec) = record();
        let store = MemStore::default();
        store.0.lock().unwrap().insert(id.clone(), rec);

        assert_eq!(
            resolve_decision(
                &store,
                &id,
                &ApprovingPrincipal("mallory".into()),
                &ApprovalScope("tool.x".into()),
                3,
                7,
                500,
            ),
            Err(ApprovalError::WrongPrincipal)
        );
        // Expired: now >= expiry.
        assert_eq!(
            resolve_decision(
                &store,
                &id,
                &ApprovingPrincipal("alice".into()),
                &ApprovalScope("tool.x".into()),
                3,
                7,
                2_000_000,
            ),
            Err(ApprovalError::Expired)
        );
    }
}
