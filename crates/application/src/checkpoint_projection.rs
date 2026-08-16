//! Deterministic, read-only projection joining checkpoint and transaction authorities.
//!
//! The projection is not a persistence authority. It refuses to pair records
//! without the trusted Session/Turn binding captured at capability admission.

use crate::workspace_checkpoint::{CheckpointFinalizeState, CheckpointId, TurnCheckpoint};
use ::contracts::change_transaction::{
    ChangeTransactionId, ChangeTransactionPhase, ChangeTransactionSnapshot, MutationCoverage,
    ValidationPlanOmission, VersionedValidationReceipt,
};
use serde::{Deserialize, Serialize};

pub const TURN_CHECKPOINT_PROJECTION_SCHEMA_V1: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSettlement {
    Open,
    Finalized,
    Aborted,
    RepairRequired,
    Accepted,
    RolledBack,
    Conflicted,
}

/// UI/read-model view derived only from authoritative checkpoint and change
/// transaction records. It deliberately carries validation evidence and
/// mutation coverage so adapters cannot infer either from transcript text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpointProjection {
    pub schema_version: u16,
    pub checkpoint_id: CheckpointId,
    pub session_id: String,
    pub turn_id: String,
    pub parent_checkpoint_id: Option<CheckpointId>,
    pub transaction_id: ChangeTransactionId,
    pub workspace_before: String,
    pub workspace_after: String,
    pub changed_paths: Vec<String>,
    pub diff_artifact_ref: Option<String>,
    pub validation_receipts: Vec<VersionedValidationReceipt>,
    pub validation_omissions: Vec<ValidationPlanOmission>,
    pub mutation_coverage: MutationCoverage,
    pub conversation_cursor: u64,
    pub plan_revision: Option<String>,
    pub settlement: CheckpointSettlement,
    pub created_at_ms: i64,
}

impl TurnCheckpointProjection {
    pub fn project(
        checkpoint: &TurnCheckpoint,
        transaction: &ChangeTransactionSnapshot,
        parent_checkpoint_id: Option<CheckpointId>,
        plan_revision: Option<String>,
    ) -> Result<Self, String> {
        if checkpoint.session_id != transaction.owner_session_id {
            return Err("checkpoint and transaction session authorities differ".into());
        }
        if transaction.owner_turn_id.as_deref() != Some(checkpoint.turn_id.as_str()) {
            return Err("transaction is not bound to the checkpoint Turn authority".into());
        }

        let settlement = match transaction.phase {
            ChangeTransactionPhase::Repair => CheckpointSettlement::RepairRequired,
            ChangeTransactionPhase::Accepted => CheckpointSettlement::Accepted,
            ChangeTransactionPhase::RolledBack => CheckpointSettlement::RolledBack,
            ChangeTransactionPhase::Conflicted => CheckpointSettlement::Conflicted,
            _ => match checkpoint.finalize_state {
                CheckpointFinalizeState::Open => CheckpointSettlement::Open,
                CheckpointFinalizeState::Finalized => CheckpointSettlement::Finalized,
                CheckpointFinalizeState::Aborted => CheckpointSettlement::Aborted,
            },
        };

        if settlement == CheckpointSettlement::Accepted {
            if checkpoint.finalize_state != CheckpointFinalizeState::Finalized {
                return Err("accepted transaction requires a finalized checkpoint".into());
            }
            if transaction.validation_receipts.is_empty()
                && transaction.validation_omissions.is_empty()
            {
                return Err(
                    "accepted checkpoint requires a validation receipt or explicit omission".into(),
                );
            }
        }

        Ok(Self {
            schema_version: TURN_CHECKPOINT_PROJECTION_SCHEMA_V1,
            checkpoint_id: checkpoint.checkpoint_id,
            session_id: checkpoint.session_id.clone(),
            turn_id: checkpoint.turn_id.clone(),
            parent_checkpoint_id,
            transaction_id: transaction.transaction_id,
            workspace_before: transaction.baseline.digest.clone(),
            workspace_after: transaction.current.digest.clone(),
            changed_paths: transaction.changed_paths.clone(),
            diff_artifact_ref: transaction.diff_artifact_ref.clone(),
            validation_receipts: transaction.validation_receipts.clone(),
            validation_omissions: transaction.validation_omissions.clone(),
            mutation_coverage: transaction.mutation_coverage,
            conversation_cursor: checkpoint.prompt_index,
            plan_revision,
            settlement,
            created_at_ms: checkpoint.created_at_ms,
        })
    }

    pub fn to_review_snapshot(&self) -> ::contracts::CheckpointReviewSnapshot {
        let mutation_coverage = match self.mutation_coverage {
            MutationCoverage::Full => ::contracts::CheckpointMutationCoverage::Full,
            MutationCoverage::BestEffort => ::contracts::CheckpointMutationCoverage::BestEffort,
            MutationCoverage::NonRollbackable => {
                ::contracts::CheckpointMutationCoverage::NonRollbackable
            }
        };
        let rollback_action = match self.mutation_coverage {
            MutationCoverage::Full => ::contracts::CheckpointRollbackAction::AutomaticAllowed,
            MutationCoverage::BestEffort => {
                ::contracts::CheckpointRollbackAction::ExplicitApprovalRequired
            }
            MutationCoverage::NonRollbackable => ::contracts::CheckpointRollbackAction::Unavailable,
        };
        let settlement = match self.settlement {
            CheckpointSettlement::Open => ::contracts::CheckpointReviewSettlement::Open,
            CheckpointSettlement::Finalized => ::contracts::CheckpointReviewSettlement::Finalized,
            CheckpointSettlement::Aborted => ::contracts::CheckpointReviewSettlement::Aborted,
            CheckpointSettlement::RepairRequired => {
                ::contracts::CheckpointReviewSettlement::RepairRequired
            }
            CheckpointSettlement::Accepted => ::contracts::CheckpointReviewSettlement::Accepted,
            CheckpointSettlement::RolledBack => ::contracts::CheckpointReviewSettlement::RolledBack,
            CheckpointSettlement::Conflicted => ::contracts::CheckpointReviewSettlement::Conflicted,
        };
        ::contracts::CheckpointReviewSnapshot {
            schema_version: self.schema_version,
            checkpoint_id: self.checkpoint_id.0.to_string(),
            session_id: self.session_id.clone(),
            task_id: format!("session:{}:task", self.session_id),
            turn_id: self.turn_id.clone(),
            parent_checkpoint_id: self.parent_checkpoint_id.map(|id| id.0.to_string()),
            workspace_before: self.workspace_before.clone(),
            workspace_after: self.workspace_after.clone(),
            changed_paths: self.changed_paths.clone(),
            diff_artifact_ref: self.diff_artifact_ref.clone(),
            validation_receipt_count: self.validation_receipts.len(),
            validation_omission_count: self.validation_omissions.len(),
            validation_receipts: self.validation_receipts.clone(),
            validation_omissions: self.validation_omissions.clone(),
            mutation_coverage,
            rollback_action,
            settlement,
            conversation_cursor: self.conversation_cursor,
            plan_revision: self.plan_revision.clone(),
            created_at_ms: self.created_at_ms,
            recovery_evidence: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_checkpoint::{FsDomainRef, WorkspaceIdentity};
    use ::contracts::change_transaction::{
        ValidationImpact, ValidationRisk, WorkspaceVersion, WorkspaceVersionBasis,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

    fn checkpoint() -> TurnCheckpoint {
        TurnCheckpoint {
            checkpoint_id: CheckpointId::new(),
            session_id: "session-1".into(),
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
            prompt_index: 7,
            workspace: WorkspaceIdentity {
                canonical_path: PathBuf::from("/workspace"),
                repo_fingerprint: None,
            },
            fs_domain: FsDomainRef {
                batch_id: Uuid::new_v4(),
                file_count: 0,
            },
            vcs_domain_ref: None,
            patch_domain_ref: None,
            runtime_checkpoint_ref: None,
            created_at_ms: 42,
            schema_version: 1,
            integrity_digest: "fixture".into(),
            finalize_state: CheckpointFinalizeState::Finalized,
        }
    }

    fn version(digest: &str) -> WorkspaceVersion {
        WorkspaceVersion {
            digest: digest.into(),
            basis: WorkspaceVersionBasis::BoundedTree,
            root: "/workspace".into(),
            head: None,
            changed_paths: vec!["src/lib.rs".into()],
        }
    }

    fn accepted_transaction() -> ChangeTransactionSnapshot {
        ChangeTransactionSnapshot {
            transaction_id: ChangeTransactionId::new(),
            owner_session_id: "session-1".into(),
            owner_turn_id: Some("turn-1".into()),
            owner_agent: None,
            root: "/workspace".into(),
            baseline: version("before"),
            current: version("after"),
            phase: ChangeTransactionPhase::Accepted,
            mutation_coverage: MutationCoverage::BestEffort,
            compensation_ref: None,
            changed_paths: vec!["src/lib.rs".into()],
            changed_ranges: vec![],
            diff_artifact_ref: Some("artifact://sha256/diff".into()),
            validation_plan: vec![],
            validation_omissions: vec![],
            validation_impact: ValidationImpact::PackageLocal,
            validation_risk: ValidationRisk::Moderate,
            validation_receipts: vec![],
            accepted_workspace_version: Some("after".into()),
            active_command: None,
            failure: None,
        }
    }

    #[test]
    fn u_chk_004_accepted_checkpoint_requires_validation_receipt_or_explicit_omission() {
        let checkpoint = checkpoint();
        let mut transaction = accepted_transaction();
        let error =
            TurnCheckpointProjection::project(&checkpoint, &transaction, None, None).unwrap_err();
        assert!(error.contains("validation receipt or explicit omission"));

        transaction
            .validation_omissions
            .push(ValidationPlanOmission {
                validation_kind: "tests".into(),
                reason: "documentation-only change".into(),
            });
        let projection =
            TurnCheckpointProjection::project(&checkpoint, &transaction, None, None).unwrap();
        assert_eq!(projection.settlement, CheckpointSettlement::Accepted);
        assert_eq!(projection.validation_omissions.len(), 1);
        assert_eq!(projection.mutation_coverage, MutationCoverage::BestEffort);
    }

    #[test]
    fn projection_rejects_unbound_or_cross_turn_transaction() {
        let checkpoint = checkpoint();
        let mut transaction = accepted_transaction();
        transaction
            .validation_omissions
            .push(ValidationPlanOmission {
                validation_kind: "tests".into(),
                reason: "explicit".into(),
            });
        transaction.owner_turn_id = Some("other-turn".into());
        assert!(TurnCheckpointProjection::project(&checkpoint, &transaction, None, None).is_err());
    }

    #[test]
    fn u_chk_005_weaker_coverage_never_projects_automatic_rollback() {
        let checkpoint = checkpoint();
        for (coverage, expected) in [
            (
                MutationCoverage::BestEffort,
                ::contracts::CheckpointRollbackAction::ExplicitApprovalRequired,
            ),
            (
                MutationCoverage::NonRollbackable,
                ::contracts::CheckpointRollbackAction::Unavailable,
            ),
        ] {
            let mut transaction = accepted_transaction();
            transaction.mutation_coverage = coverage;
            transaction
                .validation_omissions
                .push(ValidationPlanOmission {
                    validation_kind: "tests".into(),
                    reason: "explicit".into(),
                });
            let review = TurnCheckpointProjection::project(&checkpoint, &transaction, None, None)
                .unwrap()
                .to_review_snapshot();
            assert_eq!(review.rollback_action, expected);
        }
    }
}
