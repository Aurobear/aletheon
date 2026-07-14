//! Fail-closed configuration and registration for the isolated Pi coding runtime.

use crate::core::sub_agent::{SubAgentRuntime, SubAgentSpawner};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use cognit::config::PiRuntimeConfig;
use corpus::tools::subagent::{
    CommandRequest, CommandRunner, WorktreeManager, WorktreeManagerConfig,
};
use fabric::sandbox::{IsolationLevel, SandboxBackend, SandboxConfig};
use fabric::{
    AttemptEvidence, AttemptUsage, Clock, CodingJobReport, CodingJobSpec, CodingJobStatus,
    CodingNetworkPolicy, FailureClass, RuntimeFailure, RuntimeId, RuntimeResult,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const PI_CODER_RUNTIME_ID: &str = "pi-coder";

pub fn register_pi_runtime(
    spawner: &mut SubAgentSpawner,
    config: &PiRuntimeConfig,
    sandbox: Option<Arc<dyn SandboxBackend>>,
    clock: Arc<dyn Clock>,
) -> Result<bool> {
    if !config.enabled {
        return Ok(false);
    }
    let sandbox = sandbox.context("Pi runtime requires an available namespace sandbox")?;
    let runtime = PiRuntime::prepare(config, sandbox, clock)?
        .context("enabled Pi runtime did not produce a runtime")?;
    spawner
        .runtime_registry_mut()
        .register(PiRuntime::runtime_id(), Arc::new(runtime))?;
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPiConfig {
    pub executable: PathBuf,
    pub fixed_args: Vec<String>,
    pub worktree_base: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allowed_paths: Vec<PathBuf>,
    pub forbidden_paths: Vec<PathBuf>,
}

/// JSON request accepted by the stable `pi-coder` runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PiAttemptRequest {
    pub job: CodingJobSpec,
    pub task_input: String,
}

/// A configured runtime is constructible only after executable and isolation checks pass.
pub struct PiRuntime {
    config: ResolvedPiConfig,
    sandbox: Arc<dyn SandboxBackend>,
    worktrees: Arc<WorktreeManager>,
    runner: CommandRunner,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for PiRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiRuntime")
            .field("runtime_id", &PI_CODER_RUNTIME_ID)
            .field("executable", &self.config.executable)
            .field("fixed_arg_count", &self.config.fixed_args.len())
            .field("worktree_base", &self.config.worktree_base)
            .field("sandbox", &self.sandbox.name())
            .finish()
    }
}

impl PiRuntime {
    pub fn runtime_id() -> RuntimeId {
        RuntimeId(PI_CODER_RUNTIME_ID.into())
    }

    pub fn prepare(
        config: &PiRuntimeConfig,
        sandbox: Arc<dyn SandboxBackend>,
        clock: Arc<dyn Clock>,
    ) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        if !config.require_namespace_isolation {
            bail!("Pi runtime requires namespace isolation");
        }
        if config.network_enabled {
            bail!("Pi runtime network access is disabled in M4");
        }
        validate_sandbox(sandbox.as_ref())?;

        let executable = resolve_executable(config)?;
        let worktree_base = canonical_directory(&config.worktree_base, "worktree base")?;
        if config.timeout_ms == 0 || config.max_output_bytes == 0 {
            bail!("Pi runtime timeout and output limit must be nonzero");
        }
        if config.allowed_paths.is_empty() {
            bail!("Pi runtime allowed path scope must not be empty");
        }
        validate_paths(&config.allowed_paths, "allowed")?;
        validate_paths(&config.forbidden_paths, "forbidden")?;

        let resolved = ResolvedPiConfig {
            executable,
            fixed_args: config.fixed_args.clone(),
            worktree_base,
            timeout_ms: config.timeout_ms,
            max_output_bytes: config.max_output_bytes,
            allowed_paths: config.allowed_paths.clone(),
            forbidden_paths: config.forbidden_paths.clone(),
        };
        let worktrees = Arc::new(WorktreeManager::with_clock(
            WorktreeManagerConfig {
                base_dir: resolved.worktree_base.clone(),
                failed_ttl: Duration::from_secs(24 * 60 * 60),
                failed_cap: 16,
                disk_budget_bytes: 10 * 1024 * 1024 * 1024,
            },
            clock.clone(),
        )?);
        Ok(Some(Self {
            config: resolved,
            sandbox,
            worktrees,
            runner: CommandRunner,
            clock,
        }))
    }

    pub fn with_dependencies(
        config: ResolvedPiConfig,
        sandbox: Arc<dyn SandboxBackend>,
        worktrees: Arc<WorktreeManager>,
        runner: CommandRunner,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        validate_sandbox(sandbox.as_ref())?;
        Ok(Self {
            config,
            sandbox,
            worktrees,
            runner,
            clock,
        })
    }

    pub fn config(&self) -> &ResolvedPiConfig {
        &self.config
    }

    fn validate_job(&self, request: &PiAttemptRequest) -> Result<()> {
        request.job.validate()?;
        if request.task_input.trim().is_empty() {
            bail!("Pi task input must not be empty");
        }
        let command = request
            .job
            .command
            .canonicalize()
            .context("resolving coding job executable")?;
        if command != self.config.executable || request.job.args != self.config.fixed_args {
            bail!("coding job command must exactly match configured Pi argv");
        }
        if request.job.timeout_ms > self.config.timeout_ms
            || request.job.output_cap_bytes > self.config.max_output_bytes
        {
            bail!("coding job resource limits exceed Pi runtime policy");
        }
        if request.job.workspace.allowed_paths() != self.config.allowed_paths
            || request.job.workspace.forbidden_paths() != self.config.forbidden_paths
        {
            bail!("coding job path policy differs from Pi runtime policy");
        }
        if request.job.network_policy != CodingNetworkPolicy::Disabled {
            bail!("Pi coding jobs cannot request network access");
        }
        validate_sandbox(self.sandbox.as_ref())
    }

    fn failure(
        &self,
        class: FailureClass,
        message: impl Into<String>,
        retryable: bool,
        elapsed_ms: u64,
        evidence: Vec<AttemptEvidence>,
    ) -> RuntimeFailure {
        RuntimeFailure {
            class,
            message: message.into(),
            retryable,
            usage: AttemptUsage {
                elapsed_ms,
                ..Default::default()
            },
            evidence,
        }
    }
}

fn validate_sandbox(sandbox: &dyn SandboxBackend) -> Result<()> {
    if !sandbox.is_available() {
        bail!("Pi runtime sandbox '{}' is unavailable", sandbox.name());
    }
    if !matches!(
        sandbox.isolation_level(),
        IsolationLevel::Namespace | IsolationLevel::Container
    ) {
        bail!(
            "Pi runtime rejects sandbox '{}' with {:?} isolation",
            sandbox.name(),
            sandbox.isolation_level()
        );
    }
    let capabilities = sandbox.capabilities();
    if !capabilities.filesystem_isolation || !capabilities.network_isolation {
        bail!("Pi runtime sandbox lacks filesystem or network isolation");
    }
    Ok(())
}

fn resolve_executable(config: &PiRuntimeConfig) -> Result<PathBuf> {
    if config.executable.as_os_str().is_empty() {
        bail!("Pi runtime executable is missing");
    }
    let candidate = if config.executable.is_absolute() {
        config.executable.clone()
    } else {
        let trusted = config
            .trusted_executable_dir
            .as_ref()
            .context("relative Pi executable requires trusted_executable_dir")?;
        if config.executable.components().count() != 1 {
            bail!("relative Pi executable must be a single file name");
        }
        canonical_directory(trusted, "trusted executable directory")?.join(&config.executable)
    };
    let executable = candidate
        .canonicalize()
        .with_context(|| format!("resolving Pi executable: {}", candidate.display()))?;
    if !executable.is_file() {
        bail!("Pi executable is not a file: {}", executable.display());
    }
    if let Some(trusted) = &config.trusted_executable_dir {
        let trusted = canonical_directory(trusted, "trusted executable directory")?;
        if !executable.starts_with(trusted) {
            bail!("Pi executable escapes trusted executable directory");
        }
    }
    Ok(executable)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("Pi runtime {label} is missing");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving Pi runtime {label}: {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("Pi runtime {label} is not a directory");
    }
    Ok(canonical)
}

fn validate_paths(paths: &[PathBuf], label: &str) -> Result<()> {
    for path in paths {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("invalid Pi runtime {label} path: {}", path.display());
        }
    }
    Ok(())
}

#[async_trait]
impl SubAgentRuntime for PiRuntime {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String> {
        self.run_attempt(task, cancel)
            .await
            .map(|result| result.output)
            .map_err(|failure| failure.message)
    }

    async fn run_attempt(
        &self,
        task: &str,
        cancel: CancellationToken,
    ) -> std::result::Result<RuntimeResult, RuntimeFailure> {
        let started = self.clock.mono_now().0;
        let request: PiAttemptRequest = serde_json::from_str(task).map_err(|error| {
            self.failure(
                FailureClass::InvalidAssumption,
                format!("invalid Pi attempt request: {error}"),
                false,
                0,
                vec![],
            )
        })?;
        self.validate_job(&request).map_err(|error| {
            self.failure(
                FailureClass::PermissionDenied,
                format!("Pi job rejected by runtime policy: {error:#}"),
                false,
                self.clock.mono_now().0.saturating_sub(started),
                vec![],
            )
        })?;

        let lease = self
            .worktrees
            .create(
                request.job.job_id,
                request.job.workspace.repository_root(),
                &request.job.base_commit,
                cancel.clone(),
            )
            .await
            .map_err(|error| {
                self.failure(
                    FailureClass::ToolFailure,
                    format!("creating Pi worktree: {error:#}"),
                    true,
                    self.clock.mono_now().0.saturating_sub(started),
                    vec![],
                )
            })?;

        let mut sandbox_env = std::collections::HashMap::new();
        sandbox_env.insert("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into());
        sandbox_env.insert("HOME".into(), "/tmp".into());
        let sandbox_config = SandboxConfig {
            working_dir: lease.path.to_string_lossy().into_owned(),
            env_vars: sandbox_env,
        };
        let wrapped = match self.sandbox.wrap_argv(
            &self.config.executable,
            &self.config.fixed_args,
            &sandbox_config,
        ) {
            Ok(wrapped) => wrapped,
            Err(error) => {
                let _ = self
                    .worktrees
                    .finish(lease, false, CancellationToken::new())
                    .await;
                return Err(self.failure(
                    FailureClass::PermissionDenied,
                    format!("sandbox cannot provide argv-safe Pi isolation: {error:#}"),
                    false,
                    self.clock.mono_now().0.saturating_sub(started),
                    vec![],
                ));
            }
        };
        let output = match self
            .runner
            .run(
                CommandRequest {
                    program: wrapped.program,
                    args: wrapped.args,
                    working_dir: lease.path.clone(),
                    environment: wrapped.environment,
                    stdin: Some(request.task_input.as_bytes().to_vec()),
                    timeout: Duration::from_millis(request.job.timeout_ms),
                    stream_cap_bytes: request.job.output_cap_bytes,
                },
                cancel.clone(),
            )
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let _ = self
                    .worktrees
                    .finish(lease, false, CancellationToken::new())
                    .await;
                return Err(self.failure(
                    FailureClass::ToolFailure,
                    format!("executing isolated Pi command: {error}"),
                    true,
                    self.clock.mono_now().0.saturating_sub(started),
                    vec![],
                ));
            }
        };

        let snapshot = match self
            .worktrees
            .collect(&lease, &request.job.workspace, CancellationToken::new())
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self
                    .worktrees
                    .finish(lease, false, CancellationToken::new())
                    .await;
                return Err(self.failure(
                    FailureClass::PermissionDenied,
                    format!("Pi changed a forbidden or unsafe path: {error:#}"),
                    false,
                    self.clock.mono_now().0.saturating_sub(started),
                    vec![],
                ));
            }
        };

        let status = if output.cancelled {
            CodingJobStatus::Cancelled
        } else if output.timed_out {
            CodingJobStatus::TimedOut
        } else if output.exit_code == Some(0) {
            CodingJobStatus::Succeeded
        } else {
            CodingJobStatus::Failed
        };
        let report = CodingJobReport {
            job_id: request.job.job_id,
            goal_id: request.job.goal_id,
            attempt_id: request.job.attempt_id,
            base_commit: lease.base_commit.clone(),
            status,
            exit_code: output.exit_code,
            elapsed_ms: output.elapsed_ms,
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
            stdout_truncated: output.stdout_truncated,
            stderr_truncated: output.stderr_truncated,
            changed_files: snapshot.changed_files.clone(),
            diff_sha256: Some(snapshot.diff_sha256.clone()),
            diff_artifact: None,
        };
        let report_json = serde_json::to_string(&report)
            .unwrap_or_else(|error| format!(r#"{{"serialization_error":"{error}"}}"#));
        let evidence = vec![AttemptEvidence {
            kind: "coding_job_report".into(),
            summary: format!(
                "Pi {:?}: {} changed files; diff {}",
                status,
                report.changed_files.len(),
                snapshot.diff_sha256
            ),
            content: report_json,
        }];

        // Successful non-empty worktrees are retained for M5 approval/apply;
        // successful empty worktrees are disposable. Every failure is retained.
        let remove_empty_success =
            status == CodingJobStatus::Succeeded && snapshot.changed_files.is_empty();
        if let Err(error) = self
            .worktrees
            .finish(lease, remove_empty_success, CancellationToken::new())
            .await
        {
            return Err(self.failure(
                FailureClass::ToolFailure,
                format!("finalizing Pi worktree: {error:#}"),
                false,
                self.clock.mono_now().0.saturating_sub(started),
                evidence,
            ));
        }

        let usage = AttemptUsage {
            elapsed_ms: output.elapsed_ms,
            ..Default::default()
        };
        match status {
            CodingJobStatus::Succeeded => Ok(RuntimeResult {
                output: output.stdout,
                usage,
                evidence,
            }),
            CodingJobStatus::TimedOut => Err(self.failure(
                FailureClass::Timeout,
                "Pi coding attempt timed out",
                true,
                output.elapsed_ms,
                evidence,
            )),
            CodingJobStatus::Cancelled => Err(self.failure(
                FailureClass::Cancelled,
                "Pi coding attempt cancelled",
                false,
                output.elapsed_ms,
                evidence,
            )),
            _ => Err(self.failure(
                FailureClass::ToolFailure,
                format!("Pi exited with status {:?}", output.exit_code),
                true,
                output.elapsed_ms,
                evidence,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fabric::sandbox::{SandboxCapabilities, SandboxConfig, SandboxResult};
    use tempfile::TempDir;

    struct FakeSandbox {
        name: &'static str,
        level: IsolationLevel,
        available: bool,
        filesystem: bool,
        network: bool,
    }

    #[async_trait]
    impl SandboxBackend for FakeSandbox {
        fn name(&self) -> &str {
            self.name
        }

        fn isolation_level(&self) -> IsolationLevel {
            self.level
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn capabilities(&self) -> SandboxCapabilities {
            SandboxCapabilities {
                filesystem_isolation: self.filesystem,
                network_isolation: self.network,
                resource_limits: true,
                seccomp_filter: false,
                limitations: vec![],
            }
        }

        async fn execute(
            &self,
            _cmd: &str,
            _config: &SandboxConfig,
            _timeout: std::time::Duration,
        ) -> anyhow::Result<SandboxResult> {
            unreachable!("Task 4 registration must not execute Pi")
        }
    }

    fn sandbox(level: IsolationLevel) -> Arc<dyn SandboxBackend> {
        Arc::new(FakeSandbox {
            name: match level {
                IsolationLevel::Namespace => "bubblewrap",
                IsolationLevel::Process => "process",
                IsolationLevel::None => "noop",
                IsolationLevel::Container => "container",
            },
            level,
            available: true,
            filesystem: level != IsolationLevel::None,
            network: matches!(level, IsolationLevel::Namespace | IsolationLevel::Container),
        })
    }

    fn enabled_config(fixture: &TempDir) -> PiRuntimeConfig {
        let executable = fixture.path().join("pi");
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
        let worktree_base = fixture.path().join("worktrees");
        std::fs::create_dir_all(&worktree_base).unwrap();
        PiRuntimeConfig {
            enabled: true,
            executable,
            fixed_args: vec!["--mode".into(), "json".into()],
            worktree_base,
            allowed_paths: vec![PathBuf::from("crates"), PathBuf::from("Cargo.toml")],
            forbidden_paths: vec![PathBuf::from(".git"), PathBuf::from(".env")],
            ..Default::default()
        }
    }

    #[test]
    fn disabled_configuration_does_not_require_a_sandbox() {
        let mut spawner = SubAgentSpawner::new();
        assert!(!register_pi_runtime(
            &mut spawner,
            &PiRuntimeConfig::default(),
            None,
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .unwrap());
        assert!(!spawner
            .runtime_registry()
            .contains(&PiRuntime::runtime_id()));
    }

    #[test]
    fn missing_executable_and_invalid_path_policy_fail_closed() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.executable = fixture.path().join("missing");
        assert!(PiRuntime::prepare(
            &config,
            sandbox(IsolationLevel::Namespace),
            Arc::new(aletheon_kernel::chronos::TestClock::default())
        )
        .is_err());

        config = enabled_config(&fixture);
        config.allowed_paths = vec![PathBuf::from("../escape")];
        assert!(PiRuntime::prepare(
            &config,
            sandbox(IsolationLevel::Namespace),
            Arc::new(aletheon_kernel::chronos::TestClock::default())
        )
        .is_err());
    }

    #[test]
    fn noop_and_process_sandboxes_are_rejected() {
        let fixture = TempDir::new().unwrap();
        let config = enabled_config(&fixture);
        assert!(PiRuntime::prepare(
            &config,
            sandbox(IsolationLevel::None),
            Arc::new(aletheon_kernel::chronos::TestClock::default())
        )
        .is_err());
        assert!(PiRuntime::prepare(
            &config,
            sandbox(IsolationLevel::Process),
            Arc::new(aletheon_kernel::chronos::TestClock::default())
        )
        .is_err());
    }

    #[test]
    fn namespace_sandbox_is_accepted_and_debug_is_secret_free() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.fixed_args = vec!["--api-key".into(), "super-secret".into()];
        let runtime = PiRuntime::prepare(
            &config,
            sandbox(IsolationLevel::Namespace),
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(PiRuntime::runtime_id(), RuntimeId("pi-coder".into()));
        assert!(!format!("{runtime:?}").contains("super-secret"));

        let mut spawner = SubAgentSpawner::new();
        assert!(register_pi_runtime(
            &mut spawner,
            &config,
            Some(sandbox(IsolationLevel::Namespace)),
            Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
        .unwrap());
        assert!(spawner
            .runtime_registry()
            .contains(&PiRuntime::runtime_id()));
    }
}
