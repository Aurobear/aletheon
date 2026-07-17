//! Workspace checkpoint / rewind types (G4).
//!
//! Captures FS state at turn/prompt boundaries and restores it with
//! transactional semantics. Distinct from runtime checkpoints (agent/model
//! state) and from memory/event history (never erased by an FS rewind).
//!
//! This module holds the pure types plus a deterministic integrity digest and
//! identity check. Capture/finalize/restore orchestration and persistence live
//! in the Executive.
//!
//! See `docs/plans/grok/exec/G4-checkpoint-rewind.md`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(pub Uuid);

impl CheckpointId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

/// Canonical workspace identity, resisting path-alias / symlink bypass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    pub repo_fingerprint: Option<String>,
}

impl WorkspaceIdentity {
    /// Whether a restore against `other` is allowed: identities must match
    /// exactly (fail-closed defense against restoring into the wrong tree).
    pub fn matches(&self, other: &WorkspaceIdentity) -> bool {
        self == other
    }
}

/// Host-minted reference to a persisted file-snapshot batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDomainRef {
    pub batch_id: Uuid,
    pub file_count: usize,
}

/// Finalize state: even a non-Completed turn must explicitly finalize/abort;
/// no `Open` checkpoint may be left dangling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckpointFinalizeState {
    Open,
    Finalized,
    Aborted,
}

/// One logical turn checkpoint. First phase carries only the FS domain;
/// vcs/patch/runtime refs are reserved for later phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpoint {
    pub checkpoint_id: CheckpointId,
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    /// Associates the user-facing "rewind to turn N" index.
    pub prompt_index: u64,
    pub workspace: WorkspaceIdentity,
    pub fs_domain: FsDomainRef,
    pub vcs_domain_ref: Option<String>,
    pub patch_domain_ref: Option<String>,
    pub runtime_checkpoint_ref: Option<String>,
    pub created_at_ms: i64,
    pub schema_version: u32,
    pub finalize_state: CheckpointFinalizeState,
}

impl TurnCheckpoint {
    /// Deterministic integrity digest over the identity-bearing fields (not the
    /// mutable finalize_state). Stable across process restarts (sha256).
    pub fn integrity_digest(&self) -> String {
        #[derive(Serialize)]
        struct Material<'a> {
            checkpoint_id: &'a CheckpointId,
            session_id: &'a str,
            thread_id: &'a str,
            turn_id: &'a str,
            prompt_index: u64,
            workspace: &'a WorkspaceIdentity,
            fs_domain: &'a FsDomainRef,
            schema_version: u32,
        }
        let material = Material {
            checkpoint_id: &self.checkpoint_id,
            session_id: &self.session_id,
            thread_id: &self.thread_id,
            turn_id: &self.turn_id,
            prompt_index: self.prompt_index,
            workspace: &self.workspace,
            fs_domain: &self.fs_domain,
            schema_version: self.schema_version,
        };
        let encoded = serde_json::to_vec(&material).unwrap_or_default();
        let digest = Sha256::digest(&encoded);
        format!("{digest:x}")
    }
}

/// One captured file entry (mirrors FileSnap semantics).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointFileEntry {
    pub path: PathBuf,
    /// `None` = file did not exist at capture (restore should delete it).
    pub content: Option<String>,
}

/// Transactional restore outcome. Partial failure must be explainable and must
/// never masquerade as success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestoreOutcome {
    Completed,
    /// Identity check failed — nothing was modified.
    IdentityMismatch,
    /// Untracked changes could not be protected — aborted, nothing modified.
    UnprotectedChangesAbort,
    /// Core FS restore failed — checkpoints preserved for retry.
    FsRestoreFailed {
        detail: String,
    },
    /// Core succeeded but a later stage (index/hunk) partially failed.
    Partial {
        detail: String,
    },
}

impl RestoreOutcome {
    /// Whether future checkpoints may be truncated (only after a fully
    /// successful core restore).
    pub fn allows_truncation(&self) -> bool {
        matches!(self, RestoreOutcome::Completed)
    }
}

/// Captured-file-count upper bound.
pub const MAX_CHECKPOINT_FILES: usize = 2048;

#[cfg(test)]
mod tests {
    use super::*;

    fn ck() -> TurnCheckpoint {
        TurnCheckpoint {
            checkpoint_id: CheckpointId(Uuid::nil()),
            session_id: "s1".to_string(),
            thread_id: "t1".to_string(),
            turn_id: "turn1".to_string(),
            prompt_index: 2,
            workspace: WorkspaceIdentity {
                canonical_path: PathBuf::from("/home/u/project"),
                repo_fingerprint: Some("git:abc".to_string()),
            },
            fs_domain: FsDomainRef {
                batch_id: Uuid::nil(),
                file_count: 3,
            },
            vcs_domain_ref: None,
            patch_domain_ref: None,
            runtime_checkpoint_ref: None,
            created_at_ms: 1000,
            schema_version: 1,
            finalize_state: CheckpointFinalizeState::Open,
        }
    }

    #[test]
    fn integrity_digest_is_deterministic() {
        let a = ck();
        let b = ck();
        assert_eq!(a.integrity_digest(), b.integrity_digest());
    }

    #[test]
    fn integrity_digest_ignores_finalize_state() {
        let mut a = ck();
        let d1 = a.integrity_digest();
        a.finalize_state = CheckpointFinalizeState::Finalized;
        assert_eq!(
            a.integrity_digest(),
            d1,
            "finalize_state must not affect digest"
        );
    }

    #[test]
    fn integrity_digest_changes_with_content() {
        let a = ck();
        let mut b = ck();
        b.prompt_index = 99;
        assert_ne!(a.integrity_digest(), b.integrity_digest());
    }

    #[test]
    fn identity_match_is_exact() {
        let a = ck();
        let mut other = a.workspace.clone();
        assert!(a.workspace.matches(&other));
        other.canonical_path = PathBuf::from("/home/u/other");
        assert!(!a.workspace.matches(&other));
    }

    #[test]
    fn only_completed_allows_truncation() {
        assert!(RestoreOutcome::Completed.allows_truncation());
        for o in [
            RestoreOutcome::IdentityMismatch,
            RestoreOutcome::UnprotectedChangesAbort,
            RestoreOutcome::FsRestoreFailed { detail: "x".into() },
            RestoreOutcome::Partial { detail: "x".into() },
        ] {
            assert!(!o.allows_truncation(), "{o:?} must not allow truncation");
        }
    }

    #[test]
    fn checkpoint_serde_roundtrip() {
        let c = ck();
        let json = serde_json::to_string(&c).unwrap();
        let back: TurnCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
