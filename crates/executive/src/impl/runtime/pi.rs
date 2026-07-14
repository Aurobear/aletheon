//! Fail-closed configuration and registration for the isolated Pi coding runtime.

use crate::core::sub_agent::{SubAgentRuntime, SubAgentSpawner};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use cognit::config::PiRuntimeConfig;
use fabric::sandbox::{IsolationLevel, SandboxBackend};
use fabric::{AttemptUsage, FailureClass, RuntimeFailure, RuntimeId, RuntimeResult};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const PI_CODER_RUNTIME_ID: &str = "pi-coder";

pub fn register_pi_runtime(
    spawner: &mut SubAgentSpawner,
    config: &PiRuntimeConfig,
    sandbox: Option<Arc<dyn SandboxBackend>>,
) -> Result<bool> {
    if !config.enabled {
        return Ok(false);
    }
    let sandbox = sandbox.context("Pi runtime requires an available namespace sandbox")?;
    let runtime = PiRuntime::prepare(config, sandbox)?
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

/// A configured runtime is constructible only after executable and isolation checks pass.
pub struct PiRuntime {
    config: ResolvedPiConfig,
    sandbox: Arc<dyn SandboxBackend>,
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

        Ok(Some(Self {
            config: ResolvedPiConfig {
                executable,
                fixed_args: config.fixed_args.clone(),
                worktree_base,
                timeout_ms: config.timeout_ms,
                max_output_bytes: config.max_output_bytes,
                allowed_paths: config.allowed_paths.clone(),
                forbidden_paths: config.forbidden_paths.clone(),
            },
            sandbox,
        }))
    }

    pub fn config(&self) -> &ResolvedPiConfig {
        &self.config
    }
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
        _task: &str,
        _cancel: CancellationToken,
    ) -> std::result::Result<RuntimeResult, RuntimeFailure> {
        Err(RuntimeFailure {
            class: FailureClass::ToolFailure,
            message: "Pi coding execution is not initialized".into(),
            retryable: false,
            usage: AttemptUsage::default(),
            evidence: vec![],
        })
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
        assert!(!register_pi_runtime(&mut spawner, &PiRuntimeConfig::default(), None).unwrap());
        assert!(!spawner
            .runtime_registry()
            .contains(&PiRuntime::runtime_id()));
    }

    #[test]
    fn missing_executable_and_invalid_path_policy_fail_closed() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.executable = fixture.path().join("missing");
        assert!(PiRuntime::prepare(&config, sandbox(IsolationLevel::Namespace)).is_err());

        config = enabled_config(&fixture);
        config.allowed_paths = vec![PathBuf::from("../escape")];
        assert!(PiRuntime::prepare(&config, sandbox(IsolationLevel::Namespace)).is_err());
    }

    #[test]
    fn noop_and_process_sandboxes_are_rejected() {
        let fixture = TempDir::new().unwrap();
        let config = enabled_config(&fixture);
        assert!(PiRuntime::prepare(&config, sandbox(IsolationLevel::None)).is_err());
        assert!(PiRuntime::prepare(&config, sandbox(IsolationLevel::Process)).is_err());
    }

    #[test]
    fn namespace_sandbox_is_accepted_and_debug_is_secret_free() {
        let fixture = TempDir::new().unwrap();
        let mut config = enabled_config(&fixture);
        config.fixed_args = vec!["--api-key".into(), "super-secret".into()];
        let runtime = PiRuntime::prepare(&config, sandbox(IsolationLevel::Namespace))
            .unwrap()
            .unwrap();
        assert_eq!(PiRuntime::runtime_id(), RuntimeId("pi-coder".into()));
        assert!(!format!("{runtime:?}").contains("super-secret"));

        let mut spawner = SubAgentSpawner::new();
        assert!(register_pi_runtime(
            &mut spawner,
            &config,
            Some(sandbox(IsolationLevel::Namespace))
        )
        .unwrap());
        assert!(spawner
            .runtime_registry()
            .contains(&PiRuntime::runtime_id()));
    }
}
