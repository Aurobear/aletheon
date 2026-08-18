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

mod apply;
pub use apply::*;
mod service;
pub use service::*;

/// Durable receipt for one approved apply operation. This is an Application
/// result contract; persistence adapters store it without interpreting scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalApplyReceipt {
    pub approval_id: contracts::ApprovalId,
    pub operation_id: contracts::OperationId,
    pub goal_id: contracts::GoalId,
    pub success: bool,
    pub applied_head: Option<String>,
    pub diff_sha256: String,
    pub changed_paths: Vec<std::path::PathBuf>,
    pub error: Option<String>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalApplyOperation {
    pub approval_id: contracts::ApprovalId,
    pub operation_id: contracts::OperationId,
    pub status: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalApplyClaim {
    Claimed(ApprovalApplyOperation),
    Existing(ApprovalApplyOperation),
}

#[derive(Debug, Clone)]
pub struct ApprovalCreateCommand {
    pub subject: contracts::ApprovalSubject,
    pub risk: contracts::ApprovalRisk,
    pub summary: String,
    pub artifacts: Vec<contracts::ApprovalArtifactRef>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
}

/// Goal-side consumer port for requesting a durable approval.
pub trait GoalApprovalPort: Send + Sync {
    fn create(&self, command: ApprovalCreateCommand)
        -> Result<contracts::ApprovalSnapshot, String>;
}

pub use contracts::CodingJobId;

pub trait TemporaryArtifactHandle: Send + Sync {
    fn path(&self) -> &std::path::Path;
}

pub trait TemporaryArtifactStore: Send + Sync {
    fn write(&self, prefix: &str, bytes: &[u8])
        -> Result<Box<dyn TemporaryArtifactHandle>, String>;
}

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

/// Durable key for a transient session-scoped capability grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedGrantKey {
    pub principal_id: contracts::PrincipalId,
    pub thread_id: contracts::ThreadId,
    pub tool: String,
    pub path_root: String,
    pub subject_version: u32,
    pub subject_sha256: String,
}

/// Consumer-owned persistence port for transient approval grants.
///
/// The complete key prevents a tool-level approval from widening into a path
/// grant and binds path grants to the exact subject version and digest.
pub trait ScopedApprovalGrantStore: Send + Sync {
    fn clear(&self) -> anyhow::Result<()>;
    fn grant(&self, key: &ScopedGrantKey, expires_at_ms: i64) -> anyhow::Result<()>;
    fn has_tool_grant(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        now_ms: i64,
    ) -> anyhow::Result<bool>;
    fn path_roots(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        subject_version: u32,
        subject_sha256: &str,
        now_ms: i64,
    ) -> anyhow::Result<Vec<String>>;
}

/// Process-local implementation used by unit tests and isolated composition.
#[derive(Default)]
pub struct InMemoryScopedApprovalGrantStore(std::sync::Mutex<Vec<(ScopedGrantKey, i64)>>);

impl ScopedApprovalGrantStore for InMemoryScopedApprovalGrantStore {
    fn clear(&self) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        Ok(())
    }

    fn grant(&self, key: &ScopedGrantKey, expires_at_ms: i64) -> anyhow::Result<()> {
        let mut grants = self.0.lock().unwrap_or_else(|error| error.into_inner());
        grants.retain(|(existing, _)| existing != key);
        grants.push((key.clone(), expires_at_ms));
        Ok(())
    }

    fn has_tool_grant(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let grants = self.0.lock().unwrap_or_else(|error| error.into_inner());
        Ok(grants.iter().any(|(key, expiry)| {
            key.principal_id == *principal_id
                && key.thread_id == *thread_id
                && key.tool == tool
                && key.path_root.is_empty()
                && *expiry > now_ms
        }))
    }

    fn path_roots(
        &self,
        principal_id: &contracts::PrincipalId,
        thread_id: &contracts::ThreadId,
        tool: &str,
        subject_version: u32,
        subject_sha256: &str,
        now_ms: i64,
    ) -> anyhow::Result<Vec<String>> {
        let grants = self.0.lock().unwrap_or_else(|error| error.into_inner());
        Ok(grants
            .iter()
            .filter(|(key, expiry)| {
                key.principal_id == *principal_id
                    && key.thread_id == *thread_id
                    && key.tool == tool
                    && !key.path_root.is_empty()
                    && key.subject_version == subject_version
                    && key.subject_sha256 == subject_sha256
                    && *expiry > now_ms
            })
            .map(|(key, _)| key.path_root.clone())
            .collect())
    }
}

#[async_trait::async_trait]
pub trait ManagedWorktreeCleaner: Send + Sync {
    async fn cleanup(
        &self,
        job_id: CodingJobId,
        repository_root: &std::path::Path,
        worktree: &std::path::Path,
    ) -> anyhow::Result<()>;
}
