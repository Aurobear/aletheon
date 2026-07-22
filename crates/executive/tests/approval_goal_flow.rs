use async_trait::async_trait;
use base64::Engine;
use executive::approval::ApprovalRepository;
use executive::goal::{
    AttemptExecutor, AttemptRequest, CodingVerifier, GoalCoordinator, ObjectiveStore, RetryPolicy,
};
use executive::application::coding_runtime::CodingAttemptRequest;
const TEST_CODING_RUNTIME_ID: &str = "fake-coding-runtime";
use executive::service::verification::{VerificationCheckKind, VerificationContext};
use fabric::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{NamedTempFile, TempDir};
use tokio_util::sync::CancellationToken;

struct Clock50;
impl Clock for Clock50 {
    fn wall_now(&self) -> WallTime {
        WallTime(50_000)
    }
    fn mono_now(&self) -> MonoTime {
        MonoTime(50_000)
    }
}

struct Verifier {
    passed: bool,
    calls: AtomicUsize,
}
#[async_trait]
impl CodingVerifier for Verifier {
    async fn verify_coding_attempt(
        &self,
        c: &VerificationContext,
        _: CancellationToken,
    ) -> Result<VerificationReport, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(VerificationReport {
            job_id: c.job_id,
            goal_id: c.goal_id,
            attempt_id: c.attempt_id,
            passed: self.passed,
            checks: VerificationCheckKind::REQUIRED
                .into_iter()
                .map(|kind| VerificationCheck {
                    name: kind.as_str().into(),
                    severity: VerificationSeverity::Required,
                    passed: self.passed,
                    timed_out: false,
                    cancelled: false,
                    summary: if self.passed {
                        "passed".into()
                    } else {
                        "failed".into()
                    },
                    evidence: vec![],
                })
                .collect(),
            risk_summary: vec![],
            started_at_ms: 50_000,
            ended_at_ms: 50_001,
        })
    }
}

struct Executor {
    base: PathBuf,
    calls: AtomicUsize,
}
#[async_trait]
impl AttemptExecutor for Executor {
    fn is_available(&self, id: &RuntimeId) -> bool {
        id.0 == TEST_CODING_RUNTIME_ID
    }
    async fn run_once(
        &self,
        _: &RuntimeId,
        task: &str,
        _: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let r: CodingAttemptRequest = serde_json::from_str(task).unwrap();
        let relative = PathBuf::from(format!("job-{}", r.job.job_id.0));
        std::fs::create_dir_all(self.base.join(&relative)).unwrap();
        let diff = b"diff --git a/src/lib.rs b/src/lib.rs\n";
        let hash = format!("{:x}", Sha256::digest(diff));
        let report = CodingJobReport {
            job_id: r.job.job_id,
            goal_id: r.job.goal_id,
            attempt_id: r.job.attempt_id,
            base_commit: r.job.base_commit,
            status: CodingJobStatus::Succeeded,
            exit_code: Some(0),
            elapsed_ms: 1,
            stdout: "done".into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            changed_files: vec![],
            diff_sha256: Some(hash),
            diff_artifact: None,
        };
        Ok(RuntimeResult {
            output: "done".into(),
            usage: AttemptUsage::default(),
            evidence: evidence(&report, &relative, diff),
        })
    }
}

fn evidence(report: &CodingJobReport, relative: &Path, diff: &[u8]) -> Vec<AttemptEvidence> {
    vec![
        AttemptEvidence {
            kind: "coding_job_report".into(),
            summary: "report".into(),
            content: serde_json::to_string(report).unwrap(),
        },
        AttemptEvidence {
            kind: "coding_worktree_ref".into(),
            summary: "worktree".into(),
            content: relative.to_string_lossy().into_owned(),
        },
        AttemptEvidence {
            kind: "coding_diff_base64".into(),
            summary: "diff".into(),
            content: base64::engine::general_purpose::STANDARD.encode(diff),
        },
        AttemptEvidence {
            kind: "coding_capability_audit".into(),
            summary: "audit".into(),
            content:
                r#"{"audit_present":true,"observed_capabilities":[],"allowed_capabilities":[]}"#
                    .into(),
        },
    ]
}

struct Harness {
    db: NamedTempFile,
    repo: TempDir,
    worktrees: TempDir,
    store: Arc<Mutex<ObjectiveStore>>,
    goal: GoalId,
    exec: Arc<Executor>,
}
impl Harness {
    fn new() -> Self {
        let db = NamedTempFile::new().unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let worktrees = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(ObjectiveStore::open(db.path()).unwrap()));
        let goal = store
            .lock()
            .unwrap()
            .create_goal(
                &PrincipalId("owner".into()),
                "s",
                "project",
                &GoalSpec {
                    original_intent: "apply safe code".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: GoalBudget {
                        max_input_tokens: 10_000,
                        max_output_tokens: 10_000,
                        max_cost_usd: None,
                        max_attempts: 10,
                        deadline_ms: None,
                    },
                },
            )
            .unwrap();
        store
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
        let exec = Arc::new(Executor {
            base: worktrees.path().into(),
            calls: AtomicUsize::new(0),
        });
        Self {
            db,
            repo,
            worktrees,
            store,
            goal: goal.id,
            exec,
        }
    }
    fn request(&self, job: CodingJobId) -> AttemptRequest {
        let version = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal)
            .unwrap()
            .unwrap()
            .version;
        let workspace = WorkspaceBoundary::new(
            self.repo.path(),
            vec![PathBuf::from("src")],
            vec![PathBuf::from(".git")],
        )
        .unwrap();
        let task = serde_json::to_string(&CodingAttemptRequest {
            job: CodingJobSpec {
                job_id: job,
                goal_id: self.goal,
                attempt_id: AttemptId::new(),
                workspace,
                base_commit: "abcdef0123456789".into(),
                command: "pi".into(),
                args: vec![],
                timeout_ms: 1000,
                output_cap_bytes: 4096,
                network_policy: CodingNetworkPolicy::Disabled,
            },
            task_input: "implement".into(),
        })
        .unwrap();
        AttemptRequest {
            goal_id: self.goal,
            expected_version: version,
            sequence: 1,
            runtime_id: RuntimeId(TEST_CODING_RUNTIME_ID.into()),
            escalation_runtime_id: None,
            role: CognitiveRole::Worker,
            task,
            estimated_usage: AttemptUsage::default(),
        }
    }
    fn coordinator(
        &self,
        passed: bool,
        approvals: bool,
    ) -> (executive::goal::AttemptCoordinator, Arc<Verifier>) {
        let verifier = Arc::new(Verifier {
            passed,
            calls: AtomicUsize::new(0),
        });
        let goal = GoalCoordinator::new(self.store.clone());
        let base = goal
            .coding_attempt_coordinator(
                self.exec.clone(),
                Arc::new(Clock50),
                RetryPolicy::default(),
                verifier.clone(),
                self.worktrees.path(),
            )
            .unwrap();
        let result = if approvals {
            base.with_approval_repository(Arc::new(Mutex::new(
                ApprovalRepository::open(self.db.path()).unwrap(),
            )))
            .unwrap()
        } else {
            base
        };
        (result, verifier)
    }
    fn running_again(&self) {
        let g = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal)
            .unwrap()
            .unwrap();
        self.store
            .lock()
            .unwrap()
            .transition_goal(
                self.goal,
                g.version,
                GoalState::Ready,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        let g = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal)
            .unwrap()
            .unwrap();
        self.store
            .lock()
            .unwrap()
            .transition_goal(
                self.goal,
                g.version,
                GoalState::Running,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
    }
    fn pending(&self) -> Vec<ApprovalSnapshot> {
        ApprovalRepository::open(self.db.path())
            .unwrap()
            .list_pending(&PrincipalId("owner".into()), 50_000)
            .unwrap()
    }
    fn artifact_path(&self, job: CodingJobId) -> PathBuf {
        let name = self.db.path().file_name().unwrap().to_string_lossy();
        self.db
            .path()
            .with_file_name(format!("{name}.artifacts"))
            .join("coding-diffs")
            .join(format!("{}.diff", job.0))
    }
}

#[tokio::test]
async fn verified_diff_creates_one_hash_bound_apply_approval() {
    let h = Harness::new();
    let job = CodingJobId::new();
    let (c, v) = h.coordinator(true, true);
    let req = h.request(job);
    let out = c
        .execute_one(req.clone(), CancellationToken::new())
        .await
        .unwrap();
    let executive::goal::AttemptCoordinationOutcome::Succeeded { goal, .. } = out else {
        panic!()
    };
    assert_eq!(goal.state, GoalState::AwaitingHuman);
    let pending = h.pending();
    assert_eq!(pending.len(), 1);
    let a = &pending[0];
    assert_eq!(a.category, ApprovalCategory::ApplyCode);
    assert_eq!(a.subject.attributes["base_commit"], "abcdef0123456789");
    assert_eq!(a.subject.attributes["diff_sha256"].len(), 64);
    assert_eq!(a.subject.attributes["verification_sha256"].len(), 64);
    assert_eq!(a.subject.allowed_scope, vec![PathBuf::from("src")]);
    assert_eq!(a.subject.apply_target, Some(PathBuf::from(".")));
    let duplicate = c.execute_one(req, CancellationToken::new()).await.unwrap();
    assert!(
        matches!(duplicate,executive::goal::AttemptCoordinationOutcome::Succeeded{ref goal,..} if goal.state==GoalState::AwaitingHuman)
    );
    assert_eq!(h.pending().len(), 1);
    assert_eq!(h.exec.calls.load(Ordering::SeqCst), 1);
    assert_eq!(v.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn verification_failure_cannot_create_approval() {
    let h = Harness::new();
    let (c, _) = h.coordinator(false, true);
    let _ = c
        .execute_one(h.request(CodingJobId::new()), CancellationToken::new())
        .await
        .unwrap();
    assert!(h.pending().is_empty());
}

#[tokio::test]
async fn restart_between_verification_and_approval_creation_recovers_without_reexecution() {
    let h = Harness::new();
    let job = CodingJobId::new();
    let req = h.request(job);
    let (c, _) = h.coordinator(true, false);
    let _ = c
        .execute_one(req.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert!(h.pending().is_empty());
    h.running_again();
    let (c2, v2) = h.coordinator(true, true);
    let out = c2.execute_one(req, CancellationToken::new()).await.unwrap();
    assert!(
        matches!(out,executive::goal::AttemptCoordinationOutcome::Succeeded{ref goal,..} if goal.state==GoalState::AwaitingHuman)
    );
    assert_eq!(h.exec.calls.load(Ordering::SeqCst), 1);
    assert_eq!(v2.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.pending().len(), 1);
}

#[tokio::test]
async fn missing_or_tampered_diff_artifact_cannot_create_approval() {
    for remove in [true, false] {
        let h = Harness::new();
        let job = CodingJobId::new();
        let req = h.request(job);
        let (c, _) = h.coordinator(true, false);
        let _ = c
            .execute_one(req.clone(), CancellationToken::new())
            .await
            .unwrap();
        h.running_again();
        if remove {
            std::fs::remove_file(h.artifact_path(job)).unwrap()
        } else {
            std::fs::write(h.artifact_path(job), "tampered").unwrap()
        };
        let (c2, _) = h.coordinator(true, true);
        assert!(c2.execute_one(req, CancellationToken::new()).await.is_err());
        assert!(h.pending().is_empty());
    }
}

#[tokio::test]
async fn approval_survives_restart_before_delivery() {
    let h = Harness::new();
    let (c, _) = h.coordinator(true, true);
    let _ = c
        .execute_one(h.request(CodingJobId::new()), CancellationToken::new())
        .await
        .unwrap();
    let before = h.pending();
    drop(c);
    let reopened = ApprovalRepository::open(h.db.path()).unwrap();
    let after = reopened
        .list_pending(&PrincipalId("owner".into()), 50_000)
        .unwrap();
    assert_eq!(before, after);
}
