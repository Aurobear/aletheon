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
    /// SHA-256 over every immutable checkpoint field and the captured file
    /// entries.  This value is host-computed and must be verified before a
    /// checkpoint is loaded or restored.
    pub integrity_digest: String,
    pub finalize_state: CheckpointFinalizeState,
}

impl TurnCheckpoint {
    /// Calculate the deterministic integrity digest.
    ///
    /// `finalize_state` is deliberately excluded because finalization is the
    /// sole permitted mutation after insertion.  The persisted digest itself
    /// is also excluded.  All restore-bearing data, including file contents,
    /// is covered.
    pub fn calculate_integrity_digest(&self, files: &[CheckpointFileEntry]) -> String {
        #[derive(Serialize)]
        struct Material<'a> {
            checkpoint_id: &'a CheckpointId,
            session_id: &'a str,
            thread_id: &'a str,
            turn_id: &'a str,
            prompt_index: u64,
            workspace: &'a WorkspaceIdentity,
            fs_domain: &'a FsDomainRef,
            vcs_domain_ref: &'a Option<String>,
            patch_domain_ref: &'a Option<String>,
            runtime_checkpoint_ref: &'a Option<String>,
            created_at_ms: i64,
            schema_version: u32,
            files: &'a [CheckpointFileEntry],
        }
        let material = Material {
            checkpoint_id: &self.checkpoint_id,
            session_id: &self.session_id,
            thread_id: &self.thread_id,
            turn_id: &self.turn_id,
            prompt_index: self.prompt_index,
            workspace: &self.workspace,
            fs_domain: &self.fs_domain,
            vcs_domain_ref: &self.vcs_domain_ref,
            patch_domain_ref: &self.patch_domain_ref,
            runtime_checkpoint_ref: &self.runtime_checkpoint_ref,
            created_at_ms: self.created_at_ms,
            schema_version: self.schema_version,
            files,
        };
        let encoded = serde_json::to_vec(&material)
            .expect("checkpoint integrity material contains only serializable fields");
        let digest = Sha256::digest(&encoded);
        format!("{digest:x}")
    }

    /// Seal a newly-created checkpoint before persistence.
    pub fn seal_integrity(&mut self, files: &[CheckpointFileEntry]) {
        self.integrity_digest = self.calculate_integrity_digest(files);
    }

    /// Verify persisted checkpoint metadata and snapshot contents.
    pub fn verify_integrity(&self, files: &[CheckpointFileEntry]) -> bool {
        !self.integrity_digest.is_empty()
            && self.integrity_digest == self.calculate_integrity_digest(files)
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
            integrity_digest: String::new(),
            finalize_state: CheckpointFinalizeState::Open,
        }
    }

    #[test]
    fn integrity_digest_is_deterministic() {
        let a = ck();
        let b = ck();
        assert_eq!(
            a.calculate_integrity_digest(&[]),
            b.calculate_integrity_digest(&[])
        );
    }

    #[test]
    fn integrity_digest_ignores_finalize_state() {
        let mut a = ck();
        let d1 = a.calculate_integrity_digest(&[]);
        a.finalize_state = CheckpointFinalizeState::Finalized;
        assert_eq!(a.calculate_integrity_digest(&[]), d1);
    }

    #[test]
    fn integrity_digest_changes_with_content() {
        let a = ck();
        let mut b = ck();
        b.prompt_index = 99;
        assert_ne!(
            a.calculate_integrity_digest(&[]),
            b.calculate_integrity_digest(&[])
        );
    }

    #[test]
    fn integrity_digest_covers_snapshot_contents() {
        let mut checkpoint = ck();
        let files = vec![CheckpointFileEntry {
            path: PathBuf::from("src/lib.rs"),
            content: Some("safe".into()),
        }];
        checkpoint.seal_integrity(&files);
        assert!(checkpoint.verify_integrity(&files));

        let mut tampered = files;
        tampered[0].content = Some("tampered".into());
        assert!(!checkpoint.verify_integrity(&tampered));
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
