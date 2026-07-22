use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use executive::application::coding_runtime::CodingAttemptRequest;
use executive::application::verification::{
    ArchitecturePolicy, VerificationService, VerificationServiceConfig,
};
use executive::approval::{
    ApplyCoordinationOutcome, ApplyCoordinatorConfig, ApprovalDecision, ApprovalRepository,
    ApprovalResolutionContext, ManagedWorktreeCleaner,
};
use executive::goal::{
    AttemptCoordinationOutcome, AttemptExecutor, AttemptRequest, GoalCoordinator, ObjectiveStore,
    RetryPolicy,
};
use executive::testing::coding_runtime::PI_CODER_RUNTIME_ID;
use fabric::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
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

struct FixedCodingExecutor {
    worktree_base: PathBuf,
    tamper_hash: bool,
}

#[async_trait]
impl AttemptExecutor for FixedCodingExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        runtime_id.0 == PI_CODER_RUNTIME_ID
    }

    async fn run_once(
        &self,
        _: &RuntimeId,
        task: &str,
        _: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        let request: CodingAttemptRequest = serde_json::from_str(task).unwrap();
        let relative = PathBuf::from(format!("job-{}", request.job.job_id.0));
        let worktree = self.worktree_base.join(&relative);
        git(
            request.job.workspace.repository_root(),
            &[
                "worktree",
                "add",
                "--detach",
                worktree.to_str().unwrap(),
                &request.job.base_commit,
            ],
        );
        std::fs::write(worktree.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
        let diff = git_bytes(&worktree, &["diff", "--binary", "--", "src/lib.rs"]);
        let mut diff_sha256 = hex_sha256(&diff);
        if self.tamper_hash {
            diff_sha256 = "0".repeat(64);
        }
        let report = CodingJobReport {
            job_id: request.job.job_id,
            goal_id: request.job.goal_id,
            attempt_id: request.job.attempt_id,
            base_commit: request.job.base_commit,
            status: CodingJobStatus::Succeeded,
            exit_code: Some(0),
            elapsed_ms: 1,
            stdout: "fixed executor changed src/lib.rs".into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            changed_files: vec![ChangedFile {
                path: PathBuf::from("src/lib.rs"),
                kind: ChangedFileKind::Modified,
                before_bytes: 25,
                after_bytes: 25,
                content_sha256: hex_sha256(b"pub fn value() -> u8 { 2 }\n"),
            }],
            diff_sha256: Some(diff_sha256),
            diff_artifact: Some(
                PathBuf::from("coding-diffs").join(format!("{}.diff", request.job.job_id.0)),
            ),
        };
        let evidence = vec![
            AttemptEvidence { kind: "coding_job_report".into(), summary: "fixed coding report".into(), content: serde_json::to_string(&report).unwrap() },
            AttemptEvidence { kind: "coding_worktree_ref".into(), summary: "managed worktree".into(), content: relative.to_string_lossy().into_owned() },
            AttemptEvidence { kind: "coding_diff_base64".into(), summary: "bounded diff".into(), content: base64::engine::general_purpose::STANDARD.encode(diff) },
            AttemptEvidence { kind: "coding_capability_audit".into(), summary: "fixed executor audit".into(), content: r#"{"audit_present":true,"observed_capabilities":["filesystem_isolation","network_isolation"],"allowed_capabilities":["filesystem_isolation","network_isolation"],"unavailable_capabilities":["resource_limits","seccomp_filter"]}"#.into() },
        ];
        Ok(RuntimeResult {
            output: "implemented".into(),
            usage: AttemptUsage::default(),
            evidence,
        })
    }
}

#[derive(Default)]
struct Cleaner;
#[async_trait]
impl ManagedWorktreeCleaner for Cleaner {
    async fn cleanup(&self, _: CodingJobId, _: &Path, worktree: &Path) -> Result<()> {
        if worktree.exists() {
            std::fs::remove_dir_all(worktree)?;
        }
        Ok(())
    }
}

struct Fixture {
    _temp: TempDir,
    repo: PathBuf,
    worktrees: PathBuf,
    store: Arc<Mutex<ObjectiveStore>>,
    approvals: Arc<Mutex<ApprovalRepository>>,
    goal_id: GoalId,
    base_commit: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let worktrees = temp.path().join("worktrees");
        let db = temp.path().join("state.db");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(&worktrees).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "e2e@test.invalid"]);
        git(&repo, &["config", "user.name", "E2E Test"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        let base_commit = git_text(&repo, &["rev-parse", "HEAD"]);
        let store = Arc::new(Mutex::new(ObjectiveStore::open(&db).unwrap()));
        let goal = store
            .lock()
            .unwrap()
            .create_goal(
                &PrincipalId("owner".into()),
                "session",
                "project",
                &GoalSpec {
                    original_intent: "change value to two".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec!["src/lib.rs returns 2".into()],
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
        let approvals = Arc::new(Mutex::new(ApprovalRepository::open(&db).unwrap()));
        Self {
            _temp: temp,
            repo,
            worktrees,
            store,
            approvals,
            goal_id: goal.id,
            base_commit,
        }
    }

    fn request(&self, job_id: CodingJobId) -> AttemptRequest {
        let goal = self
            .store
            .lock()
            .unwrap()
            .get_goal(self.goal_id)
            .unwrap()
            .unwrap();
        let job = CodingJobSpec {
            job_id,
            goal_id: self.goal_id,
            attempt_id: AttemptId::new(),
            workspace: WorkspaceBoundary::new(
                &self.repo,
                vec![PathBuf::from("src")],
                vec![PathBuf::from(".git"), PathBuf::from(".env")],
            )
            .unwrap(),
            base_commit: self.base_commit.clone(),
            command: PathBuf::from("pi"),
            args: vec![],
            timeout_ms: 5_000,
            output_cap_bytes: 1024 * 1024,
            network_policy: CodingNetworkPolicy::Disabled,
        };
        AttemptRequest {
            goal_id: self.goal_id,
            expected_version: goal.version,
            sequence: 1,
            runtime_id: RuntimeId(PI_CODER_RUNTIME_ID.into()),
            escalation_runtime_id: None,
            role: CognitiveRole::Worker,
            task: serde_json::to_string(&CodingAttemptRequest {
                job,
                task_input: "change value".into(),
            })
            .unwrap(),
            estimated_usage: AttemptUsage::default(),
        }
    }

    fn verifier(&self, pass: bool) -> Arc<VerificationService> {
        let command = self
            ._temp
            .path()
            .join(if pass { "verify-pass" } else { "verify-fail" });
        std::fs::write(
            &command,
            if pass {
                "#!/bin/sh\nexit 0\n"
            } else {
                "#!/bin/sh\nexit 9\n"
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Arc::new(
            VerificationService::new(VerificationServiceConfig {
                cargo_program: command.canonicalize().unwrap(),
                git_program: which::which("git").unwrap().canonicalize().unwrap(),
                compile_args: vec!["check".into()],
                relevant_test_args: vec![vec!["test".into()]],
                command_timeout: Duration::from_secs(5),
                output_cap_bytes: 64 * 1024,
                environment: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                architecture: ArchitecturePolicy::default(),
            })
            .unwrap(),
        )
    }

    async fn run_goal(
        &self,
        pass_verify: bool,
        tamper_hash: bool,
    ) -> Result<AttemptCoordinationOutcome, executive::goal::AttemptCoordinatorError> {
        GoalCoordinator::new(self.store.clone())
            .approval_coding_attempt_coordinator(
                Arc::new(FixedCodingExecutor {
                    worktree_base: self.worktrees.clone(),
                    tamper_hash,
                }),
                Arc::new(TestClock),
                RetryPolicy::default(),
                self.verifier(pass_verify),
                &self.worktrees,
                self.approvals.clone(),
            )
            .unwrap()
            .execute_one(self.request(CodingJobId::new()), CancellationToken::new())
            .await
    }
}

#[tokio::test]
async fn disposable_goal_verifies_approves_applies_and_settles_once() {
    let fixture = Fixture::new();
    let outcome = fixture.run_goal(true, false).await.unwrap();
    assert!(matches!(
        outcome,
        AttemptCoordinationOutcome::Succeeded { .. }
    ));
    let approval = fixture
        .approvals
        .lock()
        .unwrap()
        .list_pending(&PrincipalId("owner".into()), 10_000)
        .unwrap()
        .pop()
        .unwrap();
    fixture
        .approvals
        .lock()
        .unwrap()
        .resolve(
            approval.id,
            approval.version,
            &ApprovalResolutionContext {
                principal_id: PrincipalId("owner".into()),
                channel: "local_rpc".into(),
            },
            ApprovalDecision::Approve,
            10_001,
        )
        .unwrap();
    let kernel = Arc::new(::kernel::KernelRuntime::with_clock(Arc::new(TestClock)));
    let owner = kernel.spawn_process(SpawnSpec::default()).await.unwrap().id;
    let coordinator = GoalCoordinator::new(fixture.store.clone())
        .approved_apply_coordinator(
            fixture.approvals.clone(),
            kernel,
            Arc::new(TestClock),
            ApplyCoordinatorConfig {
                worktree_base: fixture.worktrees.clone(),
                timeout: Duration::from_secs(5),
            },
            Arc::new(Cleaner),
        )
        .unwrap();
    assert!(matches!(
        coordinator
            .coordinate(approval.id, owner, CancellationToken::new())
            .await
            .unwrap(),
        ApplyCoordinationOutcome::Applied(_)
    ));
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("src/lib.rs")).unwrap(),
        "pub fn value() -> u8 { 2 }\n"
    );
    assert_eq!(
        fixture
            .store
            .lock()
            .unwrap()
            .get_goal(fixture.goal_id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Completed
    );
    assert!(matches!(
        coordinator
            .coordinate(approval.id, owner, CancellationToken::new())
            .await
            .unwrap(),
        ApplyCoordinationOutcome::Recovered(_)
    ));
}

#[tokio::test]
async fn hash_mismatch_and_real_verifier_failure_never_create_approval() {
    for (pass_verify, tamper_hash) in [(true, true), (false, false)] {
        let fixture = Fixture::new();
        let outcome = fixture.run_goal(pass_verify, tamper_hash).await;
        if tamper_hash {
            assert!(outcome.is_err());
        } else {
            assert!(matches!(
                outcome.unwrap(),
                AttemptCoordinationOutcome::Failed { .. }
            ));
        }
        assert!(fixture
            .approvals
            .lock()
            .unwrap()
            .list_pending(&PrincipalId("owner".into()), 10_000)
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(fixture.repo.join("src/lib.rs")).unwrap(),
            "pub fn value() -> u8 { 1 }\n"
        );
    }
}

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap()
        .success());
}
fn git_bytes(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}
fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(repo, args))
        .unwrap()
        .trim()
        .into()
}
fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
