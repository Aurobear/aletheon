use async_trait::async_trait;
use base64::Engine;
use executive::r#impl::goal::{
    AttemptCoordinationOutcome, AttemptExecutor, AttemptRequest, CodingVerifier, GoalCoordinator,
    ObjectiveStore, RetryPolicy,
};
use executive::r#impl::runtime::{PiAttemptRequest, PI_CODER_RUNTIME_ID};
use executive::service::verification::{VerificationCheckKind, VerificationContext};
use fabric::{
    AttemptEvidence, AttemptId, AttemptUsage, Clock, CodingJobId, CodingJobReport, CodingJobSpec,
    CodingJobStatus, CodingNetworkPolicy, CognitiveRole, GoalBudget, GoalId, GoalSpec, GoalState,
    PrincipalId, RuntimeFailure, RuntimeId, RuntimeResult, VerificationCheck, VerificationReport,
    VerificationSeverity, WorkspaceBoundary,
};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{NamedTempFile, TempDir};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct TestClock;
impl Clock for TestClock {
    fn wall_now(&self) -> fabric::WallTime {
        fabric::WallTime(50_000)
    }
    fn mono_now(&self) -> fabric::MonoTime {
        fabric::MonoTime(50_000)
    }
}

enum VerifyResult {
    Report(bool, bool),
    Error(&'static str),
}
struct FakeVerifier {
    results: Mutex<VecDeque<VerifyResult>>,
    calls: AtomicUsize,
}
#[async_trait]
impl CodingVerifier for FakeVerifier {
    async fn verify_coding_attempt(
        &self,
        context: &VerificationContext,
        _cancel: CancellationToken,
    ) -> Result<VerificationReport, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self
            .results
            .lock()
            .unwrap()
            .pop_front()
            .expect("queued result")
        {
            VerifyResult::Error(error) => Err(error.into()),
            VerifyResult::Report(passed, advisory_warning) => {
                let mut checks: Vec<_> = VerificationCheckKind::REQUIRED
                    .into_iter()
                    .map(|kind| VerificationCheck {
                        name: kind.as_str().into(),
                        severity: VerificationSeverity::Required,
                        passed,
                        timed_out: false,
                        cancelled: false,
                        summary: if passed {
                            format!("{} passed", kind.as_str())
                        } else {
                            "compile failed: E0308".into()
                        },
                        evidence: if passed {
                            vec![]
                        } else {
                            vec!["error[E0308]: mismatched types".into()]
                        },
                    })
                    .collect();
                if advisory_warning {
                    checks.push(VerificationCheck {
                        name: "clippy".into(),
                        severity: VerificationSeverity::Advisory,
                        passed: false,
                        timed_out: false,
                        cancelled: false,
                        summary: "advisory clippy warning".into(),
                        evidence: vec!["warning: needless borrow".into()],
                    });
                }
                Ok(VerificationReport {
                    job_id: context.job_id,
                    goal_id: context.goal_id,
                    attempt_id: context.attempt_id,
                    passed,
                    checks,
                    risk_summary: advisory_warning
                        .then(|| "advisory warning".into())
                        .into_iter()
                        .collect(),
                    started_at_ms: 50_000,
                    ended_at_ms: 50_001,
                })
            }
        }
    }
}

struct FakePiExecutor {
    worktree_base: PathBuf,
    calls: AtomicUsize,
    task_inputs: Mutex<Vec<String>>,
}
#[async_trait]
impl AttemptExecutor for FakePiExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        runtime_id.0 == PI_CODER_RUNTIME_ID
    }
    async fn run_once(
        &self,
        _runtime_id: &RuntimeId,
        task: &str,
        _cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let request: PiAttemptRequest = serde_json::from_str(task).unwrap();
        self.task_inputs
            .lock()
            .unwrap()
            .push(request.task_input.clone());
        let relative = PathBuf::from(format!("job-{}", request.job.job_id.0));
        std::fs::create_dir_all(self.worktree_base.join(&relative)).unwrap();
        let diff = b"diff --git a/src/lib.rs b/src/lib.rs\n";
        let hash = format!("{:x}", Sha256::digest(diff));
        let report = CodingJobReport {
            job_id: request.job.job_id,
            goal_id: request.job.goal_id,
            attempt_id: request.job.attempt_id,
            base_commit: request.job.base_commit,
            status: CodingJobStatus::Succeeded,
            exit_code: Some(0),
            elapsed_ms: 4,
            stdout: "implemented".into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            changed_files: vec![],
            diff_sha256: Some(hash),
            diff_artifact: None,
        };
        Ok(RuntimeResult {
            output: "implemented".into(),
            usage: AttemptUsage::default(),
            evidence: evidence(&report, &relative, diff),
        })
    }
}

struct Harness {
    _db: NamedTempFile,
    _repo: TempDir,
    worktrees: TempDir,
    store: Arc<Mutex<ObjectiveStore>>,
    goal_id: GoalId,
    executor: Arc<FakePiExecutor>,
    verifier: Arc<FakeVerifier>,
    coordinator: executive::r#impl::goal::AttemptCoordinator,
}
impl Harness {
    fn new(results: Vec<VerifyResult>) -> Self {
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
                "session",
                "project",
                &GoalSpec {
                    original_intent: "make a safe code change".into(),
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
        let executor = Arc::new(FakePiExecutor {
            worktree_base: worktrees.path().to_owned(),
            calls: AtomicUsize::new(0),
            task_inputs: Mutex::new(vec![]),
        });
        let verifier = Arc::new(FakeVerifier {
            results: Mutex::new(results.into()),
            calls: AtomicUsize::new(0),
        });
        let coordinator = GoalCoordinator::new(store.clone())
            .coding_attempt_coordinator(
                executor.clone(),
                Arc::new(TestClock),
                RetryPolicy::default(),
                verifier.clone(),
                worktrees.path(),
            )
            .unwrap();
        Self {
            _db: db,
            _repo: repo,
            worktrees,
            store,
            goal_id: goal.id,
            executor,
            verifier,
            coordinator,
        }
    }
    fn request(&self, sequence: u32, job_id: CodingJobId) -> AttemptRequest {
        let version = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal_id)
            .unwrap()
            .unwrap()
            .version;
        let workspace = WorkspaceBoundary::new(
            self._repo.path(),
            vec![PathBuf::from("src")],
            vec![PathBuf::from(".git")],
        )
        .unwrap();
        let task = serde_json::to_string(&PiAttemptRequest {
            job: CodingJobSpec {
                job_id,
                goal_id: self.goal_id,
                attempt_id: AttemptId::new(),
                workspace,
                base_commit: "0123456789abcdef".into(),
                command: PathBuf::from("pi"),
                args: vec![],
                timeout_ms: 1000,
                output_cap_bytes: 4096,
                network_policy: CodingNetworkPolicy::Disabled,
            },
            task_input: "implement requested change".into(),
        })
        .unwrap();
        AttemptRequest {
            goal_id: self.goal_id,
            expected_version: version,
            sequence,
            runtime_id: RuntimeId(PI_CODER_RUNTIME_ID.into()),
            escalation_runtime_id: None,
            role: CognitiveRole::Worker,
            task,
            estimated_usage: AttemptUsage::default(),
        }
    }
    fn restart_running(&self) {
        let snapshot = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal_id)
            .unwrap()
            .unwrap();
        self.store
            .lock()
            .unwrap()
            .transition_goal(
                self.goal_id,
                snapshot.version,
                GoalState::Ready,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        let snapshot = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal_id)
            .unwrap()
            .unwrap();
        self.store
            .lock()
            .unwrap()
            .transition_goal(
                self.goal_id,
                snapshot.version,
                GoalState::Running,
                None,
                &serde_json::json!({}),
            )
            .unwrap();
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

#[tokio::test]
async fn compile_failure_evidence_reaches_the_next_attempt() {
    let h = Harness::new(vec![
        VerifyResult::Report(false, false),
        VerifyResult::Report(true, false),
    ]);
    let first = h
        .coordinator
        .execute_one(h.request(1, CodingJobId::new()), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(first, AttemptCoordinationOutcome::Failed { .. }));
    h.restart_running();
    let second = h
        .coordinator
        .execute_one(h.request(2, CodingJobId::new()), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(second, AttemptCoordinationOutcome::Succeeded { ref goal, .. } if goal.state == GoalState::Blocked)
    );
    assert!(h.executor.task_inputs.lock().unwrap()[1].contains("compile failed: E0308"));
}

#[tokio::test]
async fn advisory_warning_is_approval_ready_and_duplicate_call_is_idempotent() {
    let h = Harness::new(vec![VerifyResult::Report(true, true)]);
    let job_id = CodingJobId::new();
    let request = h.request(1, job_id);
    let first = h
        .coordinator
        .execute_one(request.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(first, AttemptCoordinationOutcome::Succeeded { ref goal, .. } if goal.wait_reason == Some(fabric::GoalWaitReason::ExternalEvent { key: "approval required".into() }))
    );
    let duplicate = h
        .coordinator
        .execute_one(request, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(duplicate, AttemptCoordinationOutcome::Succeeded { ref goal, .. } if goal.state == GoalState::Blocked)
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.verifier.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_after_pi_before_verification_resumes_without_pi() {
    let h = Harness::new(vec![VerifyResult::Report(true, false)]);
    let request = h.request(1, CodingJobId::new());
    let parsed: PiAttemptRequest = serde_json::from_str(&request.task).unwrap();
    let attempt = h
        .store
        .lock()
        .unwrap()
        .begin_attempt(
            h.goal_id,
            1,
            &request.runtime_id,
            CognitiveRole::Worker,
            &serde_json::json!({"runtime_request": parsed}),
        )
        .unwrap();
    let relative = PathBuf::from(format!("job-{}", parsed.job.job_id.0));
    std::fs::create_dir_all(h.worktrees.path().join(&relative)).unwrap();
    let diff = b"diff";
    let report = CodingJobReport {
        job_id: parsed.job.job_id,
        goal_id: h.goal_id,
        attempt_id: attempt.id,
        base_commit: parsed.job.base_commit.clone(),
        status: CodingJobStatus::Succeeded,
        exit_code: Some(0),
        elapsed_ms: 1,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![],
        diff_sha256: Some(format!("{:x}", Sha256::digest(diff))),
        diff_artifact: None,
    };
    let result = RuntimeResult {
        output: String::new(),
        usage: AttemptUsage::default(),
        evidence: evidence(&report, &relative, diff),
    };
    h.store
        .lock()
        .unwrap()
        .finish_attempt(attempt.id, Ok(result))
        .unwrap();
    h.store
        .lock()
        .unwrap()
        .persist_coding_job(&report, &relative, diff, 50_000)
        .unwrap();
    let outcome = h
        .coordinator
        .execute_one(request, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, AttemptCoordinationOutcome::Succeeded { ref goal, .. } if goal.state == GoalState::Blocked)
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.verifier.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_after_report_persistence_does_not_verify_twice() {
    let h = Harness::new(vec![]);
    let request = h.request(1, CodingJobId::new());
    let parsed: PiAttemptRequest = serde_json::from_str(&request.task).unwrap();
    let attempt = h
        .store
        .lock()
        .unwrap()
        .begin_attempt(
            h.goal_id,
            1,
            &request.runtime_id,
            CognitiveRole::Worker,
            &serde_json::json!({}),
        )
        .unwrap();
    let relative = PathBuf::from(format!("job-{}", parsed.job.job_id.0));
    std::fs::create_dir_all(h.worktrees.path().join(&relative)).unwrap();
    let diff = b"diff";
    let report = CodingJobReport {
        job_id: parsed.job.job_id,
        goal_id: h.goal_id,
        attempt_id: attempt.id,
        base_commit: parsed.job.base_commit.clone(),
        status: CodingJobStatus::Succeeded,
        exit_code: Some(0),
        elapsed_ms: 1,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![],
        diff_sha256: Some(format!("{:x}", Sha256::digest(diff))),
        diff_artifact: None,
    };
    h.store
        .lock()
        .unwrap()
        .finish_attempt(attempt.id, Ok(RuntimeResult::default()))
        .unwrap();
    h.store
        .lock()
        .unwrap()
        .persist_coding_job(&report, &relative, diff, 50_000)
        .unwrap();
    h.store
        .lock()
        .unwrap()
        .persist_verification_report(
            &VerificationReport {
                job_id: report.job_id,
                goal_id: h.goal_id,
                attempt_id: attempt.id,
                passed: true,
                checks: vec![],
                risk_summary: vec![],
                started_at_ms: 1,
                ended_at_ms: 2,
            },
            50_000,
        )
        .unwrap();
    let outcome = h
        .coordinator
        .execute_one(request, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, AttemptCoordinationOutcome::Succeeded { ref goal, .. } if goal.state == GoalState::Blocked)
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.verifier.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn verifier_service_error_blocks_and_is_never_approval_ready() {
    let h = Harness::new(vec![VerifyResult::Error("verifier unavailable")]);
    let outcome = h
        .coordinator
        .execute_one(h.request(1, CodingJobId::new()), CancellationToken::new())
        .await
        .unwrap();
    let AttemptCoordinationOutcome::Succeeded { goal, .. } = outcome else {
        panic!("expected blocked outcome")
    };
    assert_eq!(goal.state, GoalState::Blocked);
    assert_eq!(
        goal.wait_reason,
        Some(fabric::GoalWaitReason::ExternalEvent {
            key: "verification service error".into()
        })
    );
}
