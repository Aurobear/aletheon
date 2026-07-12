use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::Clock;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// Supported OCI container runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

impl ContainerRuntime {
    /// Binary name for this runtime.
    pub fn binary(&self) -> &'static str {
        match self {
            ContainerRuntime::Docker => "docker",
            ContainerRuntime::Podman => "podman",
        }
    }
}

/// Network isolation mode for the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    /// No network access — strongest isolation.
    None,
    /// Default bridge network.
    Bridge,
    /// Share the host network stack.
    Host,
}

impl NetworkMode {
    fn as_arg(&self) -> &'static str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Bridge => "bridge",
            NetworkMode::Host => "host",
        }
    }
}

/// Resource limits applied to the container.
#[derive(Debug, Clone)]
pub struct ContainerResourceLimits {
    /// Memory limit in megabytes.
    pub memory_mb: Option<u64>,
    /// CPU percentage limit (e.g. 50 means 0.5 cores on a single-CPU host).
    pub cpu_percent: Option<u8>,
    /// Maximum number of PIDs inside the container.
    pub pids_limit: Option<u64>,
    /// Default timeout for container execution.
    pub timeout: Duration,
}

impl Default for ContainerResourceLimits {
    fn default() -> Self {
        Self {
            memory_mb: Some(512),
            cpu_percent: Some(50),
            pids_limit: Some(256),
            timeout: Duration::from_secs(60),
        }
    }
}

/// Container-based sandbox backend — runs commands inside an OCI container.
///
/// Provides the strongest isolation: separate filesystem, network namespace,
/// and resource cgroup. Requires Docker or Podman to be installed.
pub struct ContainerBackend {
    runtime: ContainerRuntime,
    default_image: String,
    network_mode: NetworkMode,
    resource_limits: ContainerResourceLimits,
    clock: Arc<dyn Clock>,
}

impl ContainerBackend {
    /// Create a new container backend with the given runtime and defaults.
    pub fn new(
        runtime: ContainerRuntime,
        default_image: String,
        network_mode: NetworkMode,
        resource_limits: ContainerResourceLimits,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            runtime,
            default_image,
            network_mode,
            resource_limits,
            clock,
        }
    }

    /// Probe for an available container runtime and build a default backend.
    /// Returns `None` if neither Docker nor Podman is found.
    pub fn probe(clock: Arc<dyn Clock>) -> Option<Self> {
        for runtime in [ContainerRuntime::Docker, ContainerRuntime::Podman] {
            if which::which(runtime.binary()).is_ok() {
                info!(runtime = runtime.binary(), "Container runtime detected");
                return Some(Self::new(
                    runtime,
                    "ubuntu:22.04".to_string(),
                    NetworkMode::None,
                    ContainerResourceLimits::default(),
                    clock,
                ));
            }
        }
        warn!("No container runtime (docker/podman) found in PATH");
        None
    }

    /// Resolve the runtime binary path or return an error.
    fn runtime_path(&self) -> Result<String> {
        which::which(self.runtime.binary())
            .map(|p| p.to_string_lossy().to_string())
            .with_context(|| {
                format!(
                    "Container runtime '{}' not found in PATH",
                    self.runtime.binary()
                )
            })
    }

    /// Build the argument list for `docker run` / `podman run`.
    pub fn build_run_args(&self, cmd: &str, config: &SandboxConfig) -> Vec<String> {
        let mut args: Vec<String> = vec!["run".into(), "--rm".into()];

        // Network isolation
        args.push("--network".into());
        args.push(self.network_mode.as_arg().into());

        // Resource limits
        if let Some(mem) = self.resource_limits.memory_mb {
            args.push("--memory".into());
            args.push(format!("{}m", mem));
        }
        if let Some(cpu) = self.resource_limits.cpu_percent {
            args.push("--cpus".into());
            // Convert percentage to fractional cores: 50% -> 0.5
            args.push(format!("{:.2}", cpu as f64 / 100.0));
        }
        if let Some(pids) = self.resource_limits.pids_limit {
            args.push("--pids-limit".into());
            args.push(pids.to_string());
        }

        // Read-only root filesystem with writable tmpfs at working dir
        args.push("--read-only".into());
        args.push("--tmpfs".into());
        args.push(format!("{}:exec,mode=1777", config.working_dir));

        // Environment variables
        for (key, value) in &config.env_vars {
            args.push("-e".into());
            args.push(format!("{}={}", key, value));
        }

        // Image
        args.push(self.default_image.clone());

        // Shell command
        args.push("/bin/bash".into());
        args.push("-c".into());
        args.push(cmd.to_string());

        args
    }

    /// Check if the configured image is available locally; pull if not.
    async fn ensure_image(&self) -> Result<()> {
        let runtime = self.runtime_path()?;

        // Check if image exists locally
        let inspect = tokio::process::Command::new(&runtime)
            .args(["inspect", "--type=image", &self.default_image])
            .output()
            .await?;

        if inspect.status.success() {
            return Ok(());
        }

        info!(image = %self.default_image, "Pulling container image");
        let pull = tokio::process::Command::new(&runtime)
            .args(["pull", &self.default_image])
            .output()
            .await?;

        if !pull.status.success() {
            let stderr = String::from_utf8_lossy(&pull.stderr);
            anyhow::bail!(
                "Failed to pull image '{}': {}",
                self.default_image,
                stderr.trim()
            );
        }

        Ok(())
    }
}

#[async_trait]
impl SandboxBackend for ContainerBackend {
    fn name(&self) -> &str {
        "container"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Container
    }

    fn is_available(&self) -> bool {
        which::which(self.runtime.binary()).is_ok()
    }

    fn capabilities(&self) -> SandboxCapabilities {
        let mut limitations = vec![];
        if self.network_mode == NetworkMode::None {
            limitations.push("No network access (--network none)".into());
        }
        limitations.push(format!("Requires {} runtime", self.runtime.binary()));
        limitations.push(format!("Default image: {}", self.default_image));

        SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: self.network_mode == NetworkMode::None,
            resource_limits: true,
            seccomp_filter: true,
            limitations,
        }
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        let runtime = self.runtime_path()?;

        info!(
            command = cmd,
            runtime = self.runtime.binary(),
            image = %self.default_image,
            "Executing command in container sandbox"
        );

        // Ensure image is available
        self.ensure_image().await?;

        let args = self.build_run_args(cmd, config);
        let start = self.clock.mono_now();

        let result = aletheon_kernel::chronos::Timer::timeout(&*self.clock, timeout, async {
            tokio::process::Command::new(&runtime)
                .args(&args)
                .current_dir(&config.working_dir)
                .output()
                .await
        })
        .await;

        let elapsed = self.clock.mono_now().0.saturating_sub(start.0);

        match result {
            Ok(Ok(output)) => Ok(SandboxResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                backend_used: "container".to_string(),
                isolation_level: IsolationLevel::Container,
                elapsed_ms: elapsed,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Container execution failed: {}", e)),
            Err(_) => Ok(SandboxResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout.as_secs()),
                exit_code: -1,
                backend_used: "container".to_string(),
                isolation_level: IsolationLevel::Container,
                elapsed_ms: elapsed,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn default_config() -> SandboxConfig {
        SandboxConfig {
            working_dir: "/workspace".to_string(),
            env_vars: HashMap::new(),
        }
    }

    fn test_backend() -> ContainerBackend {
        ContainerBackend::new(
            ContainerRuntime::Docker,
            "ubuntu:22.04".to_string(),
            NetworkMode::None,
            ContainerResourceLimits {
                memory_mb: Some(512),
                cpu_percent: Some(50),
                pids_limit: Some(256),
                timeout: Duration::from_secs(60),
            },
            std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        )
    }

    #[test]
    fn test_capabilities_reporting() {
        let backend = test_backend();
        let caps = backend.capabilities();

        assert!(caps.filesystem_isolation);
        assert!(caps.network_isolation);
        assert!(caps.resource_limits);
        assert!(caps.seccomp_filter);
        assert!(!caps.limitations.is_empty());
    }

    #[test]
    fn test_capabilities_with_bridge_network() {
        let backend = ContainerBackend::new(
            ContainerRuntime::Docker,
            "ubuntu:22.04".to_string(),
            NetworkMode::Bridge,
            ContainerResourceLimits::default(),
            std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let caps = backend.capabilities();

        // Bridge mode does not provide network isolation
        assert!(!caps.network_isolation);
    }

    #[test]
    fn test_isolation_level() {
        let backend = test_backend();
        assert_eq!(backend.isolation_level(), IsolationLevel::Container);
        assert_eq!(backend.name(), "container");
    }

    #[test]
    fn test_runtime_binary_names() {
        assert_eq!(ContainerRuntime::Docker.binary(), "docker");
        assert_eq!(ContainerRuntime::Podman.binary(), "podman");
    }

    #[test]
    fn test_build_run_args_basic() {
        let backend = test_backend();
        let config = default_config();
        let args = backend.build_run_args("echo hello", &config);

        // Should start with: run --rm
        assert_eq!(args[0], "run");
        assert_eq!(args[1], "--rm");

        // Network mode
        let network_idx = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[network_idx + 1], "none");

        // Image and command at the end
        assert!(args.contains(&"ubuntu:22.04".to_string()));
        assert!(args.contains(&"/bin/bash".to_string()));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn test_build_run_args_resource_limits() {
        let backend = test_backend();
        let config = default_config();
        let args = backend.build_run_args("true", &config);

        // Memory limit
        let mem_idx = args.iter().position(|a| a == "--memory").unwrap();
        assert_eq!(args[mem_idx + 1], "512m");

        // CPU limit: 50% -> 0.50
        let cpu_idx = args.iter().position(|a| a == "--cpus").unwrap();
        assert_eq!(args[cpu_idx + 1], "0.50");

        // PIDs limit
        let pids_idx = args.iter().position(|a| a == "--pids-limit").unwrap();
        assert_eq!(args[pids_idx + 1], "256");
    }

    #[test]
    fn test_build_run_args_no_resource_limits() {
        let backend = ContainerBackend::new(
            ContainerRuntime::Podman,
            "alpine:latest".to_string(),
            NetworkMode::Host,
            ContainerResourceLimits {
                memory_mb: None,
                cpu_percent: None,
                pids_limit: None,
                timeout: Duration::from_secs(30),
            },
            std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        );
        let config = default_config();
        let args = backend.build_run_args("ls", &config);

        assert!(!args.contains(&"--memory".to_string()));
        assert!(!args.contains(&"--cpus".to_string()));
        assert!(!args.contains(&"--pids-limit".to_string()));

        // Host network
        let network_idx = args.iter().position(|a| a == "--network").unwrap();
        assert_eq!(args[network_idx + 1], "host");

        // Podman image
        assert!(args.contains(&"alpine:latest".to_string()));
    }

    #[test]
    fn test_build_run_args_with_env_vars() {
        let backend = test_backend();
        let mut config = default_config();
        config.env_vars.insert("FOO".to_string(), "bar".to_string());
        config
            .env_vars
            .insert("BAZ".to_string(), "qux=1".to_string());

        let args = backend.build_run_args("env", &config);

        // Find all -e flags
        let e_indices: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "-e")
            .map(|(i, _)| i)
            .collect();

        assert_eq!(e_indices.len(), 2);

        let env_values: Vec<&str> = e_indices.iter().map(|i| args[i + 1].as_str()).collect();
        assert!(env_values.contains(&"FOO=bar"));
        assert!(env_values.contains(&"BAZ=qux=1"));
    }

    #[test]
    fn test_build_run_args_read_only_root() {
        let backend = test_backend();
        let config = default_config();
        let args = backend.build_run_args("id", &config);

        assert!(args.contains(&"--read-only".to_string()));
        assert!(args.contains(&"--tmpfs".to_string()));
    }

    #[test]
    fn test_network_mode_as_arg() {
        assert_eq!(NetworkMode::None.as_arg(), "none");
        assert_eq!(NetworkMode::Bridge.as_arg(), "bridge");
        assert_eq!(NetworkMode::Host.as_arg(), "host");
    }

    #[test]
    fn test_default_resource_limits() {
        let limits = ContainerResourceLimits::default();
        assert_eq!(limits.memory_mb, Some(512));
        assert_eq!(limits.cpu_percent, Some(50));
        assert_eq!(limits.pids_limit, Some(256));
        assert_eq!(limits.timeout, Duration::from_secs(60));
    }
}
