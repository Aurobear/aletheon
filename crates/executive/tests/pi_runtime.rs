use aletheon_kernel::chronos::SystemClock;
use anyhow::Result;
use async_trait::async_trait;
use cognit::config::PiRuntimeConfig;
use executive::core::sub_agent::SubAgentRuntime;
use executive::r#impl::runtime::{PiAttemptRequest, PiRuntime};
use fabric::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxResult,
};
use fabric::{
    AttemptId, CodingJobId, CodingJobReport, CodingJobSpec, CodingJobStatus, CodingNetworkPolicy,
    FailureClass, GoalId, WorkspaceBoundary,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct ArgvSandbox {
    available: bool,
}

#[async_trait]
impl SandboxBackend for ArgvSandbox {
    fn name(&self) -> &str {
        "bubblewrap-test-double"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Namespace
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: true,
            resource_limits: true,
            seccomp_filter: false,
            limitations: vec!["test double executes argv directly".into()],
        }
    }

    fn wrap_argv(
        &self,
        program: &Path,
        args: &[String],
        _config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        Ok(SandboxCommand {
            program: program.to_owned(),
            args: args.to_vec(),
            environment: BTreeMap::new(),
        })
    }

    async fn execute(
        &self,
        _cmd: &str,
        _config: &SandboxConfig,
        _timeout: Duration,
    ) -> Result<SandboxResult> {
        unreachable!("Pi must use argv-safe wrapping and CommandRunner")
    }
}

struct Fixture {
    _temp: TempDir,
    repository: PathBuf,
    worktrees: PathBuf,
    executable: PathBuf,
    base_commit: String,
}

impl Fixture {
    fn fixed_args() -> Vec<String> {
        [
            "--mode",
            "json",
            "--no-session",
            "--no-context-files",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-approve",
            "--offline",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    fn new(script: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let repository = temp.path().join("repo");
        let worktrees = temp.path().join("worktrees");
        std::fs::create_dir_all(repository.join("src")).unwrap();
        std::fs::create_dir_all(&worktrees).unwrap();
        std::fs::write(
            repository.join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.email", "pi@test.invalid"]);
        git(&repository, &["config", "user.name", "Pi Test"]);
        git(&repository, &["add", "."]);
        git(&repository, &["commit", "-qm", "base"]);
        let base_commit = git_output(&repository, &["rev-parse", "HEAD"]);
        let executable = temp.path().join("pi-fixture");
        std::fs::write(&executable, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            _temp: temp,
            repository,
            worktrees,
            executable,
            base_commit,
        }
    }

    fn config(&self, output_cap: usize, timeout_ms: u64) -> PiRuntimeConfig {
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(&self.executable).unwrap())
        );
        PiRuntimeConfig {
            enabled: true,
            executable: self.executable.clone(),
            fixed_args: Self::fixed_args(),
            package_version: "0.0.3-test".into(),
            executable_sha256: digest,
            worktree_base: self.worktrees.clone(),
            timeout_ms,
            max_output_bytes: output_cap,
            allowed_paths: vec![PathBuf::from("src")],
            forbidden_paths: vec![PathBuf::from(".env"), PathBuf::from("src/forbidden")],
            ..Default::default()
        }
    }

    fn request(&self, output_cap: usize, timeout_ms: u64, task_input: &str) -> PiAttemptRequest {
        PiAttemptRequest {
            job: CodingJobSpec {
                job_id: CodingJobId::new(),
                goal_id: GoalId(7),
                attempt_id: AttemptId::new(),
                workspace: WorkspaceBoundary::new(
                    &self.repository,
                    vec![PathBuf::from("src")],
                    vec![PathBuf::from(".env"), PathBuf::from("src/forbidden")],
                )
                .unwrap(),
                base_commit: self.base_commit.clone(),
                command: self.executable.clone(),
                args: Self::fixed_args(),
                timeout_ms,
                output_cap_bytes: output_cap,
                network_policy: CodingNetworkPolicy::Disabled,
            },
            task_input: task_input.into(),
        }
    }

    fn runtime(&self, output_cap: usize, timeout_ms: u64) -> PiRuntime {
        PiRuntime::prepare(
            &self.config(output_cap, timeout_ms),
            Arc::new(ArgvSandbox { available: true }),
            Arc::new(SystemClock::new()),
        )
        .unwrap()
        .unwrap()
    }
}

fn git(repository: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn git_output(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn encoded(request: &PiAttemptRequest) -> String {
    serde_json::to_string(request).unwrap()
}

fn report(evidence: &[fabric::AttemptEvidence]) -> CodingJobReport {
    let item = evidence
        .iter()
        .find(|item| item.kind == "coding_job_report")
        .expect("coding report evidence");
    serde_json::from_str(&item.content).unwrap()
}

#[tokio::test]
async fn successful_attempt_isolated_from_main_and_returns_structured_report() {
    let fixture = Fixture::new(
        r##"#!/bin/sh
cat >/dev/null
printf 'pub fn value() -> u8 { 2 }\n' > src/lib.rs
printf '%s\n' \
  '{"type":"session","version":3,"id":"fixture-session"}' \
  '{"type":"agent_start"}' \
  '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"usage":{"inputTokens":11,"outputTokens":7}}}' \
  '{"type":"agent_end","messages":[]}'
"##,
    );
    let before = git_output(&fixture.repository, &["status", "--porcelain=v2"]);
    let request = fixture.request(4096, 5_000, "implement the change");
    let result = fixture
        .runtime(4096, 5_000)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap();
    let report = report(&result.evidence);
    assert_eq!(report.status, CodingJobStatus::Succeeded);
    assert_eq!(result.output, "done");
    assert_eq!(result.usage.input_tokens, 11);
    assert_eq!(result.usage.output_tokens, 7);
    assert!(result
        .evidence
        .iter()
        .any(|item| item.kind == "pi_build_identity"));
    assert_eq!(report.changed_files.len(), 1);
    assert_eq!(report.changed_files[0].path, PathBuf::from("src/lib.rs"));
    assert!(report.diff_sha256.is_some());
    assert_eq!(
        std::fs::read_to_string(fixture.repository.join("src/lib.rs")).unwrap(),
        "pub fn value() -> u8 { 1 }\n"
    );
    assert_eq!(
        git_output(&fixture.repository, &["status", "--porcelain=v2"]),
        before
    );
}

#[tokio::test]
async fn bubblewrap_blocks_an_explicit_main_worktree_mutation_attempt() {
    let fixture = Fixture::new("#!/bin/sh\nset -eu\nread target\nprintf hacked > \"$target\"\n");
    let Some(backend) =
        corpus::security::sandbox::BubblewrapBackend::probe_async(Arc::new(SystemClock::new()))
            .await
    else {
        eprintln!("bubblewrap unavailable; registration fail-closed is covered separately");
        return;
    };
    let target = fixture.repository.join("src/lib.rs");
    let request = fixture.request(4096, 5_000, &target.to_string_lossy());
    let runtime = PiRuntime::prepare(
        &fixture.config(4096, 5_000),
        Arc::new(backend),
        Arc::new(SystemClock::new()),
    )
    .unwrap()
    .unwrap();
    let error = runtime
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::ToolFailure);
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "pub fn value() -> u8 { 1 }\n"
    );
    assert_eq!(
        git_output(&fixture.repository, &["status", "--porcelain=v2"]),
        ""
    );
}

#[tokio::test]
async fn nonzero_exit_and_timeout_are_structured_and_retained() {
    let failed = Fixture::new("#!/bin/sh\necho changed > src/lib.rs\necho failure >&2\nexit 7\n");
    let request = failed.request(4096, 5_000, "fail");
    let error = failed
        .runtime(4096, 5_000)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::ToolFailure);
    assert_eq!(report(&error.evidence).exit_code, Some(7));
    assert!(failed
        .worktrees
        .join(format!("job-{}", request.job.job_id.0))
        .exists());

    let timed = Fixture::new("#!/bin/sh\nsleep 30\n");
    let request = timed.request(4096, 50, "timeout");
    let error = timed
        .runtime(4096, 50)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::Timeout);
    assert_eq!(report(&error.evidence).status, CodingJobStatus::TimedOut);
}

#[tokio::test]
async fn cancellation_kills_child_process_group() {
    let fixture =
        Fixture::new("#!/bin/sh\nsleep 30 &\nchild=$!\necho $child > src/child.pid\nwait $child\n");
    let request = fixture.request(4096, 30_000, "cancel");
    let child_pid_path = fixture
        .worktrees
        .join(format!("job-{}", request.job.job_id.0))
        .join("src/child.pid");
    let runtime = fixture.runtime(4096, 30_000);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        let encoded = encoded(&request);
        async move { runtime.run_attempt(&encoded, cancel).await }
    });
    for _ in 0..100 {
        if child_pid_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let child_pid: i32 = std::fs::read_to_string(&child_pid_path)
        .expect("child pid file")
        .trim()
        .parse()
        .unwrap();
    cancel.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.class, FailureClass::Cancelled);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!PathBuf::from(format!("/proc/{child_pid}")).exists());
}

#[tokio::test]
async fn forbidden_path_and_symlink_escape_fail_after_execution() {
    let forbidden =
        Fixture::new("#!/bin/sh\nmkdir -p src/forbidden\necho bad > src/forbidden/file\n");
    let request = forbidden.request(4096, 5_000, "forbidden");
    let error = forbidden
        .runtime(4096, 5_000)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::PermissionDenied);

    let outside = TempDir::new().unwrap();
    let target = outside.path().join("target");
    std::fs::write(&target, "outside").unwrap();
    let script = format!("#!/bin/sh\nln -s {} src/link\n", target.display());
    let symlink = Fixture::new(&script);
    let request = symlink.request(4096, 5_000, "symlink");
    let error = symlink
        .runtime(4096, 5_000)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::PermissionDenied);
}

#[tokio::test]
async fn truncated_json_stream_and_unavailable_sandbox_fail_closed() {
    let fixture =
        Fixture::new("#!/bin/sh\ni=0; while [ $i -lt 5000 ]; do printf x; i=$((i+1)); done\n");
    let request = fixture.request(128, 5_000, "large output");
    let error = fixture
        .runtime(128, 5_000)
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::ArchitectureViolation);

    let error = PiRuntime::prepare(
        &fixture.config(128, 5_000),
        Arc::new(ArgvSandbox { available: false }),
        Arc::new(SystemClock::new()),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("unavailable"));
}

#[tokio::test]
async fn goal_cannot_replace_executable_or_enable_network() {
    let fixture = Fixture::new("#!/bin/sh\necho should-not-run > src/lib.rs\n");
    let runtime = fixture.runtime(4096, 5_000);
    let mut request = fixture.request(4096, 5_000, "policy");
    request.job.command = PathBuf::from("/bin/true");
    let error = runtime
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::PermissionDenied);
    assert_eq!(
        std::fs::read_to_string(fixture.repository.join("src/lib.rs")).unwrap(),
        "pub fn value() -> u8 { 1 }\n"
    );

    request.job.command = fixture.executable.clone();
    request.job.network_policy = CodingNetworkPolicy::AllowHosts {
        hosts: vec!["example.com".into()],
    };
    let error = runtime
        .run_attempt(&encoded(&request), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.class, FailureClass::PermissionDenied);
}
