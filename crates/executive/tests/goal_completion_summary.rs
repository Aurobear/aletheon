use executive::r#impl::approval::{
    ApprovalApplyReceipt, ApprovalCreate, ApprovalDecision, ApprovalRepository,
    ApprovalResolutionContext,
};
use executive::r#impl::goal::{GoalCompletionSummary, ObjectiveStore};
use fabric::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Outcome {
    Accepted,
    Rejected,
    Revision,
    ApplyFailed,
}

struct Fixture {
    _temp: TempDir,
    db_path: PathBuf,
    store: ObjectiveStore,
    approval: ApprovalSnapshot,
    receipt: Option<ApprovalApplyReceipt>,
}

impl Fixture {
    fn new(outcome: Outcome) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("objectives.db");
        let store = ObjectiveStore::open(&db_path).unwrap();
        let goal = store
            .create_goal(
                &PrincipalId("owner".into()),
                "session",
                "project",
                &GoalSpec {
                    original_intent: "ship safe change using sk-secret-value".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: GoalBudget::default(),
                },
            )
            .unwrap();
        let goal = store
            .transition_goal(
                goal.id,
                goal.version,
                GoalState::Running,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        let attempt = store
            .begin_attempt(
                goal.id,
                1,
                &RuntimeId("pi-coder".into()),
                CognitiveRole::Worker,
                &serde_json::json!({"task":"code"}),
            )
            .unwrap();
        store
            .finish_attempt(
                attempt.id,
                Ok(RuntimeResult {
                    output: "implemented token=private-value".into(),
                    usage: AttemptUsage::default(),
                    evidence: vec![],
                }),
            )
            .unwrap();
        let job_id = CodingJobId::new();
        let diff = b"diff --git a/allowed.txt b/allowed.txt\n";
        let diff_hash = hash(diff);
        let report = CodingJobReport {
            job_id,
            goal_id: goal.id,
            attempt_id: attempt.id,
            base_commit: "0123456789abcdef".into(),
            status: CodingJobStatus::Succeeded,
            exit_code: Some(0),
            elapsed_ms: 1,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            changed_files: vec![ChangedFile {
                path: PathBuf::from("allowed.txt"),
                kind: ChangedFileKind::Modified,
                before_bytes: 7,
                after_bytes: 6,
                content_sha256: "content-hash".into(),
            }],
            diff_sha256: Some(diff_hash.clone()),
            diff_artifact: None,
        };
        let coding = store
            .persist_coding_job(
                &report,
                &PathBuf::from(format!("job-{}", job_id.0)),
                diff,
                20,
            )
            .unwrap();
        let verification = VerificationReport {
            job_id,
            goal_id: goal.id,
            attempt_id: attempt.id,
            passed: true,
            checks: vec![VerificationCheck {
                name: "cargo test".into(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "passed with Bearer hidden-token".into(),
                evidence: vec![],
            }],
            risk_summary: vec!["risk includes sk-risk-secret".into()],
            started_at_ms: 21,
            ended_at_ms: 22,
        };
        store
            .persist_verification_report(&verification, 23)
            .unwrap();
        let verification_hash = hash(&serde_json::to_vec(&verification).unwrap());
        let approvals = ApprovalRepository::open(&db_path).unwrap();
        let pending = approvals
            .create(ApprovalCreate {
                subject: ApprovalSubject {
                    category: ApprovalCategory::ApplyCode,
                    goal_id: goal.id,
                    attempt_id: Some(attempt.id),
                    job_id: Some(job_id),
                    attributes: BTreeMap::from([
                        ("base_commit".into(), report.base_commit.clone()),
                        (
                            "repository_root".into(),
                            temp.path().to_string_lossy().into_owned(),
                        ),
                        ("diff_sha256".into(), diff_hash.clone()),
                        ("verification_sha256".into(), verification_hash),
                    ]),
                    allowed_scope: vec![PathBuf::from("allowed.txt")],
                    apply_target: Some(PathBuf::from(".")),
                },
                risk: ApprovalRisk::High,
                summary: "apply verified patch".into(),
                artifacts: vec![ApprovalArtifactRef {
                    kind: "diff".into(),
                    relative_path: coding.diff_artifact_ref,
                    sha256: diff_hash.clone(),
                }],
                created_at_ms: 30,
                expires_at_ms: 10_000,
            })
            .unwrap();
        let decision = match outcome {
            Outcome::Accepted | Outcome::ApplyFailed => ApprovalDecision::Approve,
            Outcome::Rejected => ApprovalDecision::Reject { reason: None },
            Outcome::Revision => ApprovalDecision::Reject {
                reason: Some("owner requested revision token=revision-secret".into()),
            },
        };
        let mut approval = approvals
            .resolve(
                pending.id,
                pending.version,
                &ApprovalResolutionContext {
                    principal_id: PrincipalId("owner".into()),
                    channel: "local_rpc".into(),
                },
                decision,
                40,
            )
            .unwrap();
        let mut receipt = None;
        let current = store.get_goal(goal.id).unwrap().unwrap();
        match outcome {
            Outcome::Accepted | Outcome::ApplyFailed => {
                let operation_id = OperationId::new();
                approvals
                    .claim_apply(approval.id, operation_id, 41)
                    .unwrap();
                let success = matches!(outcome, Outcome::Accepted);
                let value = ApprovalApplyReceipt {
                    approval_id: approval.id,
                    operation_id,
                    goal_id: goal.id,
                    success,
                    applied_head: success.then(|| report.base_commit.clone()),
                    diff_sha256: diff_hash,
                    changed_paths: if success {
                        vec![PathBuf::from("allowed.txt")]
                    } else {
                        vec![]
                    },
                    error: (!success).then(|| "conflict token=apply-secret".into()),
                    finished_at_ms: 50,
                };
                approval = approvals.finish_apply(&value).unwrap();
                store
                    .transition_goal(
                        goal.id,
                        current.version,
                        if success {
                            GoalState::Completed
                        } else {
                            GoalState::Blocked
                        },
                        None,
                        &serde_json::json!({}),
                    )
                    .unwrap();
                receipt = Some(value);
            }
            Outcome::Rejected => {
                store
                    .transition_goal(
                        goal.id,
                        current.version,
                        GoalState::Cancelled,
                        None,
                        &serde_json::json!({}),
                    )
                    .unwrap();
            }
            Outcome::Revision => {
                let waiting = store
                    .transition_goal(
                        goal.id,
                        current.version,
                        GoalState::AwaitingHuman,
                        None,
                        &serde_json::json!({}),
                    )
                    .unwrap();
                store
                    .transition_goal(
                        goal.id,
                        waiting.version,
                        GoalState::Ready,
                        None,
                        &serde_json::json!({}),
                    )
                    .unwrap();
            }
        }
        Self {
            _temp: temp,
            db_path,
            store,
            approval,
            receipt,
        }
    }

    fn persist(&self) -> GoalCompletionSummary {
        let summary =
            GoalCompletionSummary::build(&self.store, &self.approval, self.receipt.as_ref(), 60)
                .unwrap();
        self.store
            .persist_goal_completion_summary(&summary)
            .unwrap()
    }
}

#[test]
fn accepted_summary_contains_bounded_audit_fields_and_redacts_secrets() {
    let fixture = Fixture::new(Outcome::Accepted);
    let summary = fixture.persist();
    assert_eq!(summary.final_state, "completed");
    assert_eq!(summary.attempts.len(), 1);
    assert_eq!(summary.changed_files, vec![PathBuf::from("allowed.txt")]);
    assert_eq!(summary.checks.len(), 1);
    assert!(summary.apply.as_ref().unwrap().success);
    let json = serde_json::to_string(&summary).unwrap();
    for secret in [
        "sk-secret-value",
        "private-value",
        "hidden-token",
        "sk-risk-secret",
    ] {
        assert!(!json.contains(secret));
    }
    assert!(json.contains("[REDACTED]"));
}

#[test]
fn rejected_revision_and_apply_failed_summaries_capture_final_outcome() {
    let rejected = Fixture::new(Outcome::Rejected).persist();
    assert_eq!(rejected.final_state, "cancelled");
    assert_eq!(rejected.approval.status, "rejected");
    assert!(rejected.apply.is_none());

    let revision = Fixture::new(Outcome::Revision).persist();
    assert_eq!(revision.final_state, "ready");
    assert!(revision
        .approval
        .reason
        .as_deref()
        .unwrap()
        .contains("[REDACTED]"));

    let failed = Fixture::new(Outcome::ApplyFailed).persist();
    assert_eq!(failed.final_state, "blocked");
    assert!(!failed.apply.as_ref().unwrap().success);
    assert!(failed
        .apply
        .as_ref()
        .unwrap()
        .error
        .as_deref()
        .unwrap()
        .contains("[REDACTED]"));
}

#[test]
fn persisted_summary_is_restart_safe_and_immutable() {
    let fixture = Fixture::new(Outcome::Accepted);
    let expected = fixture.persist();
    drop(fixture.store);
    let reopened = ObjectiveStore::open(&fixture.db_path).unwrap();
    let recovered = reopened
        .load_goal_completion_summary(expected.approval_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered, expected);
    assert_eq!(
        reopened.persist_goal_completion_summary(&expected).unwrap(),
        expected
    );
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
