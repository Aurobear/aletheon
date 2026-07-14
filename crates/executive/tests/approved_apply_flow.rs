use async_trait::async_trait;
use executive::r#impl::approval::{
    ApplyCoordinationOutcome, ApplyCoordinatorConfig, ApprovalCreate, ApprovalDecision,
    ApprovalRepository, ApprovalResolutionContext, ManagedWorktreeCleaner,
};
use executive::r#impl::goal::{GoalCoordinator, ObjectiveStore};
use fabric::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct TestClock;
impl Clock for TestClock {
    fn wall_now(&self) -> WallTime {
        WallTime(10_000)
    }
    fn mono_now(&self) -> MonoTime {
        MonoTime(10_000)
    }
}

#[derive(Default)]
struct Cleaner {
    calls: AtomicUsize,
}

#[async_trait]
impl ManagedWorktreeCleaner for Cleaner {
    async fn cleanup(&self, _: CodingJobId, worktree: &Path) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if worktree.exists() {
            std::fs::remove_dir_all(worktree)?;
        }
        Ok(())
    }
}

struct Fixture {
    _temp: TempDir,
    repository: PathBuf,
    worktree_base: PathBuf,
    worktree: PathBuf,
    store: Arc<Mutex<ObjectiveStore>>,
    approvals: Arc<Mutex<ApprovalRepository>>,
    approval_id: ApprovalId,
    goal_id: GoalId,
    cleaner: Arc<Cleaner>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_decision(ApprovalDecision::Approve)
    }

    fn with_decision(decision: ApprovalDecision) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("objectives.db");
        let repository = temp.path().join("target");
        let worktree_base = temp.path().join("worktrees");
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::create_dir_all(&worktree_base).unwrap();
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.email", "test@example.com"]);
        git(&repository, &["config", "user.name", "Test"]);
        std::fs::write(repository.join("allowed.txt"), "before\n").unwrap();
        git(&repository, &["add", "allowed.txt"]);
        git(&repository, &["commit", "-qm", "base"]);
        let base = output(&repository, &["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        std::fs::write(repository.join("allowed.txt"), "after\n").unwrap();
        let diff = Command::new("git")
            .args(["diff", "--binary", "--", "allowed.txt"])
            .current_dir(&repository)
            .output()
            .unwrap()
            .stdout;
        git(&repository, &["checkout", "--", "allowed.txt"]);

        let store = Arc::new(Mutex::new(ObjectiveStore::open(&db_path).unwrap()));
        let goal = store
            .lock()
            .unwrap()
            .create_goal(
                &PrincipalId("owner".into()),
                "session",
                "project",
                &GoalSpec {
                    original_intent: "apply verified code".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: GoalBudget::default(),
                },
            )
            .unwrap();
        let goal = store
            .lock()
            .unwrap()
            .transition_goal(
                goal.id,
                goal.version,
                GoalState::Running,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        let attempt = store
            .lock()
            .unwrap()
            .begin_attempt(
                goal.id,
                1,
                &RuntimeId("pi-coder".into()),
                CognitiveRole::Worker,
                &serde_json::json!({"task":"change"}),
            )
            .unwrap();
        let job_id = CodingJobId::new();
        let worktree_ref = PathBuf::from(format!("job-{}", job_id.0));
        let worktree = worktree_base.join(&worktree_ref);
        std::fs::create_dir(&worktree).unwrap();
        let diff_hash = hash(&diff);
        let report = CodingJobReport {
            job_id,
            goal_id: goal.id,
            attempt_id: attempt.id,
            base_commit: base.clone(),
            status: CodingJobStatus::Succeeded,
            exit_code: Some(0),
            elapsed_ms: 1,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            changed_files: vec![],
            diff_sha256: Some(diff_hash.clone()),
            diff_artifact: None,
        };
        store
            .lock()
            .unwrap()
            .persist_coding_job(&report, &worktree_ref, &diff, 100)
            .unwrap();
        let verification = VerificationReport {
            job_id,
            goal_id: goal.id,
            attempt_id: attempt.id,
            passed: true,
            checks: vec![VerificationCheck {
                name: "test".into(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "passed".into(),
                evidence: vec![],
            }],
            risk_summary: vec![],
            started_at_ms: 101,
            ended_at_ms: 102,
        };
        store
            .lock()
            .unwrap()
            .persist_verification_report(&verification, 103)
            .unwrap();
        let verification_hash = hash(&serde_json::to_vec(&verification).unwrap());
        let approvals = Arc::new(Mutex::new(ApprovalRepository::open(&db_path).unwrap()));
        let approval = approvals
            .lock()
            .unwrap()
            .create(ApprovalCreate {
                subject: ApprovalSubject {
                    category: ApprovalCategory::ApplyCode,
                    goal_id: goal.id,
                    attempt_id: Some(attempt.id),
                    job_id: Some(job_id),
                    attributes: BTreeMap::from([
                        ("base_commit".into(), base),
                        ("diff_sha256".into(), diff_hash.clone()),
                        ("verification_sha256".into(), verification_hash),
                    ]),
                    allowed_scope: vec![PathBuf::from("allowed.txt")],
                    apply_target: Some(PathBuf::from(".")),
                },
                risk: ApprovalRisk::High,
                summary: "apply".into(),
                artifacts: vec![ApprovalArtifactRef {
                    kind: "diff".into(),
                    relative_path: report.diff_artifact.unwrap_or_else(|| {
                        PathBuf::from("coding-diffs").join(format!("{}.diff", job_id.0))
                    }),
                    sha256: diff_hash,
                }],
                created_at_ms: 200,
                expires_at_ms: 20_000,
            })
            .unwrap();
        let approval = approvals
            .lock()
            .unwrap()
            .resolve(
                approval.id,
                approval.version,
                &ApprovalResolutionContext {
                    principal_id: PrincipalId("owner".into()),
                    channel: "local_rpc".into(),
                },
                decision,
                300,
            )
            .unwrap();
        let current = store.lock().unwrap().get_goal(goal.id).unwrap().unwrap();
        store
            .lock()
            .unwrap()
            .transition_goal(
                goal.id,
                current.version,
                GoalState::AwaitingHuman,
                Some(&GoalWaitReason::HumanInput {
                    prompt: "approve".into(),
                }),
                &serde_json::json!({}),
            )
            .unwrap();
        Self {
            _temp: temp,
            repository,
            worktree_base,
            worktree,
            store,
            approvals,
            approval_id: approval.id,
            goal_id: goal.id,
            cleaner: Arc::new(Cleaner::default()),
        }
    }

    fn coordinator(&self) -> executive::r#impl::approval::ApplyCoordinator {
        let goal = GoalCoordinator::new(self.store.clone());
        goal.approved_apply_coordinator(
            self.approvals.clone(),
            Arc::new(aletheon_kernel::operation::OperationTable::new(Arc::new(
                TestClock,
            ))),
            Arc::new(TestClock),
            ApplyCoordinatorConfig {
                repository_root: self.repository.clone(),
                worktree_base: self.worktree_base.clone(),
                timeout: Duration::from_secs(5),
            },
            self.cleaner.clone(),
        )
        .unwrap()
    }
}

#[tokio::test]
async fn approved_apply_is_consumed_once_and_completes_goal() {
    let f = Fixture::new();
    let coordinator = f.coordinator();
    let first = coordinator
        .coordinate(f.approval_id, ProcessId::new(), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(first, ApplyCoordinationOutcome::Applied(_)));
    assert_eq!(
        std::fs::read_to_string(f.repository.join("allowed.txt")).unwrap(),
        "after\n"
    );
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .get_goal(f.goal_id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Completed
    );
    assert_eq!(
        f.approvals
            .lock()
            .unwrap()
            .get(f.approval_id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Consumed
    );
    assert_eq!(f.cleaner.calls.load(Ordering::SeqCst), 1);
    assert!(!f.worktree.exists());
    assert!(f
        .store
        .lock()
        .unwrap()
        .load_goal_completion_summary(f.approval_id)
        .unwrap()
        .is_some());

    let second = coordinator
        .coordinate(f.approval_id, ProcessId::new(), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(second, ApplyCoordinationOutcome::Recovered(_)));
}

#[tokio::test]
async fn restart_recovers_claim_before_apply_and_receipt_after_apply() {
    let f = Fixture::new();
    let operation_id = OperationId::new();
    f.approvals
        .lock()
        .unwrap()
        .claim_apply(f.approval_id, operation_id, 500)
        .unwrap();
    let restarted = f.coordinator();
    assert!(matches!(
        restarted
            .coordinate(f.approval_id, ProcessId::new(), CancellationToken::new())
            .await
            .unwrap(),
        ApplyCoordinationOutcome::Applied(_)
    ));
    let after = f.coordinator();
    assert!(matches!(
        after
            .coordinate(f.approval_id, ProcessId::new(), CancellationToken::new())
            .await
            .unwrap(),
        ApplyCoordinationOutcome::Recovered(_)
    ));
}

#[tokio::test]
async fn cancelled_apply_is_receipted_blocked_and_not_reusable() {
    let f = Fixture::new();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = f
        .coordinator()
        .coordinate(f.approval_id, ProcessId::new(), cancel)
        .await
        .unwrap();
    assert!(matches!(result, ApplyCoordinationOutcome::Failed(_)));
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .get_goal(f.goal_id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Blocked
    );
    assert_eq!(
        f.approvals
            .lock()
            .unwrap()
            .get(f.approval_id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Consumed
    );
    assert!(f
        .approvals
        .lock()
        .unwrap()
        .apply_receipt(f.approval_id)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn concurrent_callbacks_schedule_only_one_apply() {
    let f = Fixture::new();
    let coordinator = Arc::new(f.coordinator());
    let (left, right) = tokio::join!(
        coordinator.coordinate(f.approval_id, ProcessId::new(), CancellationToken::new()),
        coordinator.coordinate(f.approval_id, ProcessId::new(), CancellationToken::new())
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert!(matches!(
        left,
        ApplyCoordinationOutcome::Applied(_)
            | ApplyCoordinationOutcome::DuplicateInProgress { .. }
            | ApplyCoordinationOutcome::Recovered(_)
    ));
    assert!(matches!(
        right,
        ApplyCoordinationOutcome::Applied(_)
            | ApplyCoordinationOutcome::DuplicateInProgress { .. }
            | ApplyCoordinationOutcome::Recovered(_)
    ));
    assert!(f
        .approvals
        .lock()
        .unwrap()
        .apply_receipt(f.approval_id)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn rejection_and_revision_transition_without_applying() {
    let revision = Fixture::with_decision(ApprovalDecision::Reject {
        reason: Some("owner requested revision".into()),
    });
    let result = revision
        .coordinator()
        .coordinate(
            revision.approval_id,
            ProcessId::new(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        ApplyCoordinationOutcome::Rejected {
            revision_requested: true,
            ..
        }
    ));
    assert_eq!(
        revision
            .store
            .lock()
            .unwrap()
            .get_goal(revision.goal_id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Ready
    );
    assert_eq!(
        std::fs::read_to_string(revision.repository.join("allowed.txt")).unwrap(),
        "before\n"
    );

    let rejected = Fixture::with_decision(ApprovalDecision::Reject { reason: None });
    rejected
        .coordinator()
        .coordinate(
            rejected.approval_id,
            ProcessId::new(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        rejected
            .store
            .lock()
            .unwrap()
            .get_goal(rejected.goal_id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Cancelled
    );
}

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

fn output(repo: &Path, args: &[&str]) -> String {
    let value = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(value.status.success());
    String::from_utf8(value.stdout).unwrap()
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
